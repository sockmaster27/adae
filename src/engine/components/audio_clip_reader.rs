use std::{
    cmp::min,
    error::Error,
    fmt::{Debug, Display},
    iter::zip,
    ops::{Add, AddAssign, Mul, Range, Sub, SubAssign},
    sync::Arc,
};

use rubato::{FftFixedOut, Resampler};

use crate::engine::{info::Info, utils::non_copy_array, Sample, CHANNELS};

use super::stored_audio_clip::StoredAudioClip;

pub struct AudioClipReader {
    inner: Arc<StoredAudioClip>,
    resampler: Option<FftFixedOut<Sample>>,

    /// The position in the inner clip where the resampler will draw from.
    /// If `resampler` is none, this is not used.
    inner_position: OriginalSamples,
    /// The position as it would be appear from the output,
    /// in the domain of the output sample rate.
    position: ResampledSamples,

    channel_scale_buffer: [Vec<Sample>; CHANNELS],
    /// How many frames in the resample buffer are unused (at the end)
    resample_buffer_unused: ResampledSamples,
    resample_buffer: [Vec<Sample>; CHANNELS],
    output_buffer: Vec<Sample>,
}
impl AudioClipReader {
    pub fn new(clip: Arc<StoredAudioClip>, max_buffer_size: usize, sample_rate: u32) -> Self {
        let resampler_chunk_size = 1024;

        let clip_sample_rate: usize = clip
            .sample_rate
            .try_into()
            .expect("Clip sample rate too high");

        let (resampler, max_input_size, delay) = if clip_sample_rate != sample_rate as usize {
            let r = FftFixedOut::new(
                clip_sample_rate,
                sample_rate as usize,
                resampler_chunk_size,
                1,
                CHANNELS,
            )
            .expect("Failed to create resampler");
            let m = r.input_frames_max();
            let d = ResampledSamples(r.output_delay());
            (Some(r), m, d)
        } else {
            (None, max_buffer_size, ResampledSamples(0))
        };

        let mut audio_clip_reader = AudioClipReader {
            inner: clip,
            resampler,

            inner_position: OriginalSamples(0),
            position: ResampledSamples(0),

            channel_scale_buffer: non_copy_array![vec![0.0; max_input_size]; CHANNELS],
            resample_buffer_unused: ResampledSamples(0),
            resample_buffer: non_copy_array![vec![0.0; resampler_chunk_size]; CHANNELS],
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],
        };

        audio_clip_reader.chop_delay(delay, sample_rate);
        audio_clip_reader.position = ResampledSamples(0);

        audio_clip_reader
    }

    /// Throws out the given delay from the start of the clip.
    /// Remember to set `self.position` after calling this.
    fn chop_delay(&mut self, delay: ResampledSamples, sample_rate: u32) {
        let delay: usize = delay.into();

        let max_buffer_size = self.output_buffer.len() / CHANNELS;

        let reps = delay / max_buffer_size;
        for _ in 0..reps {
            self.output(&Info {
                sample_rate,
                buffer_size: max_buffer_size,
            });
        }

        let remaining = delay % max_buffer_size;
        self.output(&Info {
            sample_rate,
            buffer_size: remaining,
        });
    }

    /// Returns the current position in samples relative to the start of the clip, within the given `sample_rate`.
    pub fn position(&self) -> usize {
        self.position.into()
    }

    /// Jump to position with sample precision relative to the start of the clip.
    ///
    /// - `sample_rate` is the sample rate of the output.
    /// - `position` is converted from `sample_rate` into the clips original sample rate.
    pub fn jump(&mut self, position: usize, sample_rate: u32) -> Result<(), JumpOutOfBounds> {
        let pos_resampled = ResampledSamples(position);
        let pos_original = pos_resampled.into_original(sample_rate, self.inner.sample_rate);

        if self.len_original() <= pos_original {
            return Err(JumpOutOfBounds);
        }

        self.inner_position = pos_original.into();

        let delay = self
            .resampler
            .as_mut()
            .map(|r| {
                r.reset();
                ResampledSamples(r.output_delay())
            })
            .unwrap_or(ResampledSamples(0));
        self.chop_delay(delay, sample_rate);

        self.resample_buffer_unused = ResampledSamples(0);

        self.position = pos_resampled;

        Ok(())
    }

    // The length of the inner clip in frames (samples per channel), converted relative to the given sample rate.
    pub fn len(&self, sample_rate: u32) -> usize {
        self.len_resampled(sample_rate).into()
    }
    fn len_resampled(&self, sample_rate: u32) -> ResampledSamples {
        self.len_original()
            .into_resampled(sample_rate, self.inner.sample_rate)
    }
    fn len_original(&self) -> OriginalSamples {
        OriginalSamples(self.inner.len())
    }

    /// Scales `range` of each channel in `input` to down to two channels and writes it to `output`.
    ///
    /// `input` should have either 1 or two channels.
    ///
    /// `output` should have exactly two channels.
    ///
    /// If the `output` buffers are longer than the range, they will be padded with zeroes.
    fn scale_channels(
        input: &Vec<Vec<Sample>>,
        input_range: Range<OriginalSamples>,
        output: &mut [&mut [Sample]; CHANNELS],
    ) {
        let input_range: Range<usize> = input_range.start.into()..input_range.end.into();

        debug_assert_eq!(output.len(), CHANNELS);

        let input_channels = input.len();
        debug_assert_ne!(input_channels, 0);

        match input_channels {
            1 => {
                output[0][..input_range.len()].copy_from_slice(&input[0][input_range.clone()]);
                output[1][..input_range.len()].copy_from_slice(&input[0][input_range.clone()]);
            }
            2 => {
                debug_assert_eq!(input[0].len(), input[1].len());
                output[0][..input_range.len()].copy_from_slice(&input[0][input_range.clone()]);
                output[1][..input_range.len()].copy_from_slice(&input[1][input_range.clone()]);
            }
            _ => {
                // This should have been caught while importing
                panic!("Clip has incompatible number of channels");
            }
        }

        output[0][input_range.len()..].fill(0.0);
        output[1][input_range.len()..].fill(0.0);
    }

    /// Take `input` consisting of a list of exactly 2 channels, and interleave them into the `output`-buffer.
    ///
    /// Like this: `[[l1, l2, l3], [r1, r2, r3]] -> [l1, r1, l2, r2, l3, r3]`
    ///
    /// `input_range` must be within the bounds of the input channels.
    ///
    /// If `output` is longer than necassary, the remaining samples are left untouched.
    fn interleave(
        input: &[Vec<Sample>],
        input_range: Range<ResampledSamples>,
        output: &mut [Sample],
    ) {
        let input_range: Range<usize> = input_range.start.into()..input_range.end.into();

        debug_assert_eq!(input.len(), CHANNELS);
        debug_assert!(input_range.clone().len() * CHANNELS <= output.len());
        debug_assert!(input_range.clone().end <= input[0].len());
        debug_assert_eq!(input[0].len(), input[1].len());

        let input_left = &input[0][input_range.clone()];
        let input_right = &input[1][input_range];
        for (i, (&left, &right)) in zip(input_left.iter(), input_right.iter()).enumerate() {
            output[i * CHANNELS] = left;
            output[i * CHANNELS + 1] = right;
        }
    }

    /// Output with resampling.
    /// This assumes that `self.resampler` is `Some`.
    fn output_resampling(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;
        let buffer_size = ResampledSamples(buffer_size);

        let resampler = self
            .resampler
            .as_mut()
            .expect("output_resampling was called on AudioClipReader without resampler");

        // self.len_original() cannot be used, since self is already borrowed by resampler
        let len_original = OriginalSamples(self.inner.len());

        let resampled_length = len_original.into_resampled(sample_rate, self.inner.sample_rate);
        let remaining = resampled_length - self.position;

        let output_size = min(buffer_size, remaining);

        let mut filled = ResampledSamples(0);
        while filled < output_size {
            let resample_chunk_size = ResampledSamples(resampler.output_frames_max());

            if self.resample_buffer_unused == ResampledSamples(0) {
                let range = self.inner_position
                    ..min(
                        self.inner_position + OriginalSamples(resampler.input_frames_next()),
                        len_original,
                    );
                self.inner_position += range.end - range.start;

                let mut c = self.channel_scale_buffer.iter_mut();
                let mut channel_scale_buffer = [
                    &mut c.next().unwrap()[..resampler.input_frames_next()],
                    &mut c.next().unwrap()[..resampler.input_frames_next()],
                ];
                Self::scale_channels(&self.inner.data, range, &mut channel_scale_buffer);

                let result = resampler.process_into_buffer(
                    &channel_scale_buffer,
                    &mut self.resample_buffer,
                    None,
                );
                debug_assert!(result.is_ok(), "Resampler error: {:?}", result);
                self.resample_buffer_unused = resample_chunk_size;
            }

            let used = resample_chunk_size - self.resample_buffer_unused;
            let input_range = used..min(used + output_size - filled, resample_chunk_size);
            let input_len = input_range.end - input_range.start;
            self.resample_buffer_unused -= input_len;
            let output_range = filled * CHANNELS..(filled + input_len) * CHANNELS;

            filled += input_len;

            Self::interleave(
                &self.resample_buffer,
                input_range,
                &mut self.output_buffer[output_range.start.into()..output_range.end.into()],
            );
        }

        self.position += output_size;
        &mut self.output_buffer[..(output_size * CHANNELS).into()]
    }

    /// Output without resampling.
    fn output_not_resampling(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate: _,
            buffer_size,
        } = *info;

        // Here ResampledSamples and OriginalSamples are exectly the same

        let buffer_size = OriginalSamples(buffer_size);
        let position = OriginalSamples(self.position.into());

        let remaining = self.len_original() - position;
        let output_size = min(buffer_size, remaining);

        let range = position..position + output_size;
        self.position += ResampledSamples(output_size.into());

        let mut c = self.channel_scale_buffer.iter_mut();
        let mut channel_scale_buffer = [
            &mut c.next().unwrap()[..output_size.into()],
            &mut c.next().unwrap()[..output_size.into()],
        ];
        Self::scale_channels(&self.inner.data, range, &mut channel_scale_buffer);
        Self::interleave(
            &self.channel_scale_buffer,
            ResampledSamples(0)..ResampledSamples(output_size.into()),
            &mut self.output_buffer,
        );

        &mut self.output_buffer[..(output_size * CHANNELS).into()]
    }

    /// Outputs to a buffer of at most the requested size (via the info parameter).
    /// If the end is reached the returned buffer is smaller than this size.
    pub fn output(&mut self, info: &Info) -> &mut [Sample] {
        if self.resampler.is_some() {
            self.output_resampling(info)
        } else {
            self.output_not_resampling(info)
        }
    }
}
impl Debug for AudioClipReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AudioClipReader {{ inner: {:?}, position(): {:?}, ... }}",
            self.inner, self.inner_position,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct JumpOutOfBounds;
impl Display for JumpOutOfBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Attempted to jump before the start or past end of audio clip data"
        )
    }
}
impl Error for JumpOutOfBounds {}

/// A number of samples in the domain of the clip's original sample rate.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct OriginalSamples(usize);
impl OriginalSamples {
    fn into_resampled(self, sample_rate: u32, clip_sample_rate: u32) -> ResampledSamples {
        let resample_ratio = sample_rate as f64 / clip_sample_rate as f64;
        ResampledSamples((self.0 as f64 * resample_ratio).ceil() as usize)
    }
}
impl Add<OriginalSamples> for OriginalSamples {
    type Output = OriginalSamples;

    fn add(self, rhs: OriginalSamples) -> Self::Output {
        OriginalSamples(self.0 + rhs.0)
    }
}
impl AddAssign<OriginalSamples> for OriginalSamples {
    fn add_assign(&mut self, rhs: OriginalSamples) {
        self.0 += rhs.0;
    }
}
impl Sub<OriginalSamples> for OriginalSamples {
    type Output = OriginalSamples;

    fn sub(self, rhs: OriginalSamples) -> Self::Output {
        OriginalSamples(self.0 - rhs.0)
    }
}
impl SubAssign for OriginalSamples {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
impl Mul<usize> for OriginalSamples {
    type Output = OriginalSamples;

    fn mul(self, rhs: usize) -> Self::Output {
        OriginalSamples(self.0 * rhs)
    }
}
impl Mul<OriginalSamples> for usize {
    type Output = OriginalSamples;

    fn mul(self, rhs: OriginalSamples) -> Self::Output {
        OriginalSamples(self * rhs.0)
    }
}
impl From<OriginalSamples> for usize {
    fn from(s: OriginalSamples) -> Self {
        s.0
    }
}

/// A number of samples in the domain of the engine's sample rate.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct ResampledSamples(usize);
impl ResampledSamples {
    fn into_original(self, sample_rate: u32, clip_sample_rate: u32) -> OriginalSamples {
        let resample_ratio = clip_sample_rate as f64 / sample_rate as f64;
        OriginalSamples((self.0 as f64 * resample_ratio).ceil() as usize)
    }
}
impl Add<ResampledSamples> for ResampledSamples {
    type Output = ResampledSamples;

    fn add(self, rhs: ResampledSamples) -> Self::Output {
        ResampledSamples(self.0 + rhs.0)
    }
}
impl AddAssign<ResampledSamples> for ResampledSamples {
    fn add_assign(&mut self, rhs: ResampledSamples) {
        self.0 += rhs.0;
    }
}
impl Sub<ResampledSamples> for ResampledSamples {
    type Output = ResampledSamples;

    fn sub(self, rhs: ResampledSamples) -> Self::Output {
        ResampledSamples(self.0 - rhs.0)
    }
}
impl SubAssign<ResampledSamples> for ResampledSamples {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
impl Mul<usize> for ResampledSamples {
    type Output = ResampledSamples;

    fn mul(self, rhs: usize) -> Self::Output {
        ResampledSamples(self.0 * rhs)
    }
}
impl Mul<ResampledSamples> for usize {
    type Output = ResampledSamples;

    fn mul(self, rhs: ResampledSamples) -> Self::Output {
        ResampledSamples(self * rhs.0)
    }
}
impl From<ResampledSamples> for usize {
    fn from(s: ResampledSamples) -> Self {
        s.0
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::utils::test_file_path;

    use super::*;

    #[test]
    fn sample_domains() {
        let sample_rate = 44_100;
        let clip_sample_rate = 22_050;

        let resampled = OriginalSamples(clip_sample_rate as usize)
            .into_resampled(sample_rate, clip_sample_rate);
        assert_eq!(resampled, ResampledSamples(sample_rate as usize));

        let original =
            ResampledSamples(sample_rate as usize).into_original(sample_rate, clip_sample_rate);
        assert_eq!(original, OriginalSamples(clip_sample_rate as usize));
    }

    #[test]
    fn output() {
        let ac = StoredAudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        for _ in 0..5 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 50,
            });

            assert_eq!(output.len(), 50 * CHANNELS);

            let ls = output[0];
            assert!(0.999 <= ls && ls <= 1.001, "Sample: {}", ls);
            let rs = output[1];
            assert!(-1.001 <= rs && rs <= -0.999, "Sample: {}", rs);

            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 50,
            });

            assert_eq!(output.len(), 50 * CHANNELS);

            let ls = output[0];
            assert!(-1.001 <= ls && ls <= -0.999, "First left sample: {}", ls);
            let rs = output[1];
            assert!(0.999 <= rs && rs <= 1.001, "First right sample: {}", rs);
        }
    }

    #[test]
    fn output_resampling() {
        let ac = StoredAudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        for _ in 0..5 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 50,
            });

            assert_eq!(output.len(), 50 * CHANNELS);

            for &mut s in output {
                assert_ne!(s, 0.0);
            }
        }
    }

    #[test]
    fn output_big_buffer() {
        let ac = StoredAudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 2050, 48_000);

        for _ in 0..2 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 2050,
            });

            assert_eq!(output.len(), 2050 * CHANNELS);

            let ls = output[0];
            assert!(0.999 <= ls && ls <= 1.001, "Sample: {}", ls);
            let rs = output[1];
            assert!(-1.001 <= rs && rs <= -0.999, "Sample: {}", rs);

            let ls = output[2050 * CHANNELS - 2];
            assert!(-1.001 <= ls && ls <= -0.999, "Sample: {}", ls);
            let rs = output[2050 * CHANNELS - 1];
            assert!(0.999 <= rs && rs <= 1.001);

            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 2050,
            });

            assert_eq!(output.len(), 2050 * CHANNELS);

            let ls = output[0];
            assert!(-1.001 <= ls && ls <= -0.999, "Sample: {}", ls);
            let rs = output[1];
            assert!(0.999 <= rs && rs <= 1.001, "Sample: {}", rs);

            let ls = output[2050 * CHANNELS - 2];
            assert!(0.999 <= ls && ls <= 1.001, "Sample: {}", ls);
            let rs = output[2050 * CHANNELS - 1];
            assert!(-1.001 <= rs && rs <= -0.999, "Sample: {}", rs);
        }
    }

    #[test]
    fn output_big_buffer_resampling() {
        let ac = StoredAudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 2050, 48_000);

        for _ in 0..2 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 2050,
            });

            assert_eq!(output.len(), 2050 * CHANNELS);

            for &mut s in output {
                assert_ne!(s, 0.0);
            }
        }
    }

    #[test]
    fn output_past_end() {
        let ac = StoredAudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 1_322_978 + 10, 48_000);

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: 1_322_978 + 10,
        });

        assert_eq!(
            output.len(),
            1_322_978 * CHANNELS,
            "Output length does not match"
        );
    }

    #[test]
    fn output_past_end_resampling() {
        let full_length = (1_322_978.0f64 * (48_000.0 / 44_100.0)).ceil() as usize;

        let ac = StoredAudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), full_length + 10, 48_000);

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: full_length + 10,
        });

        assert_eq!(
            output.len(),
            full_length * CHANNELS,
            "Output length does not match"
        );
    }

    #[test]
    fn jump() {
        let ac = StoredAudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let ls = output[0];
        assert!(0.999 <= ls && ls <= 1.001, "Sample: {}", ls);
        let rs = output[1];
        assert!(-1.001 <= rs && rs <= -0.999, "Sample: {}", rs);

        acr.jump(0, 48_000).unwrap();

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let ls = output[0];
        assert!(0.999 <= ls && ls <= 1.001, "Sample: {}", ls);
        let rs = output[1];
        assert!(-1.001 <= rs && rs <= -0.999, "Sample: {}", rs);
    }

    #[test]
    fn jump_out_of_bounds() {
        let ac = StoredAudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        assert_eq!(acr.jump(1_322_978, 48_000), Err(JumpOutOfBounds));
    }
}
