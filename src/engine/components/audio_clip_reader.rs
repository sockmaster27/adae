use std::{
    cmp::min,
    fmt::Debug,
    iter::zip,
    ops::{Add, AddAssign, Mul, Range, Sub, SubAssign},
    sync::Arc,
};

use rubato::{FftFixedOut, Resampler};
use serde::{Deserialize, Serialize};

use crate::{
    engine::{info::Info, utils::non_copy_array, Sample, CHANNELS},
    StoredAudioClipKey,
};

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
            .sample_rate()
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
            let d = ResampledSamples::new(r.output_delay());
            (Some(r), m, d)
        } else {
            (None, max_buffer_size, ResampledSamples::new(0))
        };

        let mut audio_clip_reader = AudioClipReader {
            inner: clip,
            resampler,

            inner_position: OriginalSamples::new(0),
            position: ResampledSamples::new(0),

            channel_scale_buffer: non_copy_array![vec![0.0; max_input_size]; CHANNELS],
            resample_buffer_unused: ResampledSamples::new(0),
            resample_buffer: non_copy_array![vec![0.0; resampler_chunk_size]; CHANNELS],
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],
        };

        audio_clip_reader.chop_delay(delay, sample_rate);
        audio_clip_reader.position = ResampledSamples::new(0);

        audio_clip_reader
    }

    pub fn key(&self) -> StoredAudioClipKey {
        self.inner.key()
    }

    pub fn sample_rate_original(&self) -> u32 {
        self.inner.sample_rate()
    }

    pub fn channels_original(&self) -> usize {
        self.inner.channels()
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
    pub fn position(&self) -> ResampledSamples {
        self.position
    }

    /// Jump to position with sample precision relative to the start of the clip, in the domain of the clip's original sample rate.
    ///
    /// - `sample_rate` is the sample rate of the output.
    /// - If the position is after the end of the clip, the position is set to the end of the clip.
    pub fn jump_original(&mut self, position: OriginalSamples, sample_rate: u32) {
        let desired_pos_original = position;
        let desired_pos_resampled =
            desired_pos_original.into_resampled(sample_rate, self.inner.sample_rate());

        self.jump(desired_pos_original, desired_pos_resampled, sample_rate)
    }

    fn jump(
        &mut self,
        desired_pos_original: OriginalSamples,
        desired_pos_resampled: ResampledSamples,
        sample_rate: u32,
    ) {
        let (pos_resample, pos_original) = if self.len_original() < desired_pos_original {
            (self.len_resampled(sample_rate), self.len_original())
        } else {
            (desired_pos_resampled, desired_pos_original)
        };

        let delay = self
            .resampler
            .as_mut()
            .map(|r| {
                r.reset();
                ResampledSamples::new(r.output_delay())
            })
            .unwrap_or(ResampledSamples::new(0));
        self.chop_delay(delay, sample_rate);

        self.resample_buffer_unused = ResampledSamples::new(0);

        self.position = pos_resample;
        self.inner_position = pos_original;
    }

    // The length of the inner clip in frames (samples per channel), converted relative to the given sample rate.
    pub fn len_resampled(&self, sample_rate: u32) -> ResampledSamples {
        self.len_original()
            .into_resampled(sample_rate, self.inner.sample_rate())
    }
    // The length of the inner clip in frames (samples per channel), before resampling.
    pub fn len_original(&self) -> OriginalSamples {
        OriginalSamples::new(self.inner.length())
    }

    /// Scales `range` of each channel in `input` to down to two channels and writes it to `output`.
    ///
    /// `input` should have either 1 or two channels.
    ///
    /// `output` should have exactly two channels.
    ///
    /// If the `output` buffers are longer than the range, they will be padded with zeroes.
    fn scale_channels(
        input: &[Vec<Sample>],
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
        debug_assert!(input_range.len() * CHANNELS <= output.len());
        debug_assert!(input_range.end <= input[0].len());
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
        let buffer_size = ResampledSamples::new(buffer_size);

        let resampler = self
            .resampler
            .as_mut()
            .expect("output_resampling was called on AudioClipReader without resampler");

        // self.len_original() cannot be used, since self is already borrowed by resampler
        let len_original = OriginalSamples::new(self.inner.length());

        let resampled_length = len_original.into_resampled(sample_rate, self.inner.sample_rate());
        let remaining = resampled_length - self.position;

        let output_size = min(buffer_size, remaining);

        let mut filled = ResampledSamples::new(0);
        while filled < output_size {
            let resample_chunk_size = ResampledSamples::new(resampler.output_frames_max());

            if self.resample_buffer_unused == ResampledSamples::new(0) {
                let range = self.inner_position
                    ..min(
                        self.inner_position + OriginalSamples::new(resampler.input_frames_next()),
                        len_original,
                    );
                self.inner_position += range.end - range.start;

                let mut c = self.channel_scale_buffer.iter_mut();
                let mut channel_scale_buffer = [
                    &mut c.next().unwrap()[..resampler.input_frames_next()],
                    &mut c.next().unwrap()[..resampler.input_frames_next()],
                ];
                Self::scale_channels(self.inner.data(), range, &mut channel_scale_buffer);

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

        let buffer_size = OriginalSamples::new(buffer_size);
        let position = OriginalSamples::new(self.position.into());

        let remaining = self.len_original() - position;
        let output_size = min(buffer_size, remaining);

        let range = position..position + output_size;
        self.position += ResampledSamples::new(output_size.into());

        let mut c = self.channel_scale_buffer.iter_mut();
        let mut channel_scale_buffer = [
            &mut c.next().unwrap()[..output_size.into()],
            &mut c.next().unwrap()[..output_size.into()],
        ];
        Self::scale_channels(self.inner.data(), range, &mut channel_scale_buffer);
        Self::interleave(
            &self.channel_scale_buffer,
            ResampledSamples::new(0)..ResampledSamples::new(output_size.into()),
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

    /// Outputs the raw data of the clip without resampling.
    /// Returns a slice of channels, where each channel is a vector of samples.
    pub fn output_raw(&self) -> &[Vec<Sample>] {
        self.inner.data()
    }
}
impl Debug for AudioClipReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioClipReader")
            .field("inner", &self.inner)
            .field("inner_position", &self.inner_position)
            .field("position", &self.position)
            .finish_non_exhaustive()
    }
}

/// A number of samples in the domain of the clip's original sample rate.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize, Hash)]
pub struct OriginalSamples(usize);
impl OriginalSamples {
    pub fn new(samples: usize) -> Self {
        OriginalSamples(samples)
    }

    pub fn into_resampled(
        self,
        resampled_sample_rate: u32,
        original_sample_rate: u32,
    ) -> ResampledSamples {
        let resample_ratio = resampled_sample_rate as f64 / original_sample_rate as f64;
        ResampledSamples::new((self.0 as f64 * resample_ratio).ceil() as usize)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        OriginalSamples::new(self.0.saturating_sub(rhs.0))
    }
}
impl Add<OriginalSamples> for OriginalSamples {
    type Output = OriginalSamples;

    fn add(self, rhs: OriginalSamples) -> Self::Output {
        OriginalSamples::new(self.0 + rhs.0)
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
        OriginalSamples::new(self.0 - rhs.0)
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
        OriginalSamples::new(self.0 * rhs)
    }
}
impl Mul<OriginalSamples> for usize {
    type Output = OriginalSamples;

    fn mul(self, rhs: OriginalSamples) -> Self::Output {
        OriginalSamples::new(self * rhs.0)
    }
}
impl From<OriginalSamples> for usize {
    fn from(s: OriginalSamples) -> Self {
        s.0
    }
}

/// A number of samples in the domain of the engine's sample rate.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Serialize, Deserialize, Hash)]
pub struct ResampledSamples(usize);
impl ResampledSamples {
    pub fn new(samples: usize) -> Self {
        ResampledSamples(samples)
    }

    pub fn into_original(self, sample_rate: u32, clip_sample_rate: u32) -> OriginalSamples {
        let resample_ratio = clip_sample_rate as f64 / sample_rate as f64;
        OriginalSamples::new((self.0 as f64 * resample_ratio).ceil() as usize)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        ResampledSamples::new(self.0.saturating_sub(rhs.0))
    }
}
impl Add<ResampledSamples> for ResampledSamples {
    type Output = ResampledSamples;

    fn add(self, rhs: ResampledSamples) -> Self::Output {
        ResampledSamples::new(self.0 + rhs.0)
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
        ResampledSamples::new(self.0 - rhs.0)
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
        ResampledSamples::new(self.0 * rhs)
    }
}
impl Mul<ResampledSamples> for usize {
    type Output = ResampledSamples;

    fn mul(self, rhs: ResampledSamples) -> Self::Output {
        ResampledSamples::new(self * rhs.0)
    }
}
impl From<ResampledSamples> for usize {
    fn from(s: ResampledSamples) -> Self {
        s.0
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::utils::{key_generator::Key, test_file_path};

    use super::*;

    #[test]
    fn sample_domains() {
        let sample_rate = 44_100;
        let clip_sample_rate = 22_050;

        let resampled = OriginalSamples::new(clip_sample_rate as usize)
            .into_resampled(sample_rate, clip_sample_rate);
        assert_eq!(resampled, ResampledSamples::new(sample_rate as usize));

        let original = ResampledSamples::new(sample_rate as usize)
            .into_original(sample_rate, clip_sample_rate);
        assert_eq!(original, OriginalSamples::new(clip_sample_rate as usize));
    }

    #[test]
    fn output() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("48000 16-bit.wav"),
        )
        .unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        for _ in 0..5 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 50,
            });

            assert_eq!(output.len(), 50 * CHANNELS);

            let ls = output[0];
            assert!((0.999..=1.001).contains(&ls), "Sample: {}", ls);
            let rs = output[1];
            assert!((-1.001..=-0.999).contains(&rs), "Sample: {}", rs);

            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 50,
            });

            assert_eq!(output.len(), 50 * CHANNELS);

            let ls = output[0];
            assert!((-1.001..=-0.999).contains(&ls), "First left sample: {}", ls);
            let rs = output[1];
            assert!((0.999..=1.001).contains(&rs), "First right sample: {}", rs);
        }
    }

    #[test]
    fn output_resampling() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("44100 16-bit.wav"),
        )
        .unwrap();
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
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("48000 16-bit.wav"),
        )
        .unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 2050, 48_000);

        for _ in 0..2 {
            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 2050,
            });

            assert_eq!(output.len(), 2050 * CHANNELS);

            let ls = output[0];
            assert!((0.999..=1.001).contains(&ls), "Sample: {}", ls);
            let rs = output[1];
            assert!((-1.001..=-0.999).contains(&rs), "Sample: {}", rs);

            let ls = output[2050 * CHANNELS - 2];
            assert!((-1.001..=-0.999).contains(&ls), "Sample: {}", ls);
            let rs = output[2050 * CHANNELS - 1];
            assert!((0.999..=1.001).contains(&rs));

            let output = acr.output(&Info {
                sample_rate: 48_000,
                buffer_size: 2050,
            });

            assert_eq!(output.len(), 2050 * CHANNELS);

            let ls = output[0];
            assert!((-1.001..=-0.999).contains(&ls), "Sample: {}", ls);
            let rs = output[1];
            assert!((0.999..=1.001).contains(&rs), "Sample: {}", rs);

            let ls = output[2050 * CHANNELS - 2];
            assert!((0.999..=1.001).contains(&ls), "Sample: {}", ls);
            let rs = output[2050 * CHANNELS - 1];
            assert!((-1.001..=-0.999).contains(&rs), "Sample: {}", rs);
        }
    }

    #[test]
    fn output_big_buffer_resampling() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("44100 16-bit.wav"),
        )
        .unwrap();
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
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("48000 16-bit.wav"),
        )
        .unwrap();
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

        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("44100 16-bit.wav"),
        )
        .unwrap();
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
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("48000 16-bit.wav"),
        )
        .unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let ls = output[0];
        assert!((0.999..=1.001).contains(&ls), "Sample: {}", ls);
        let rs = output[1];
        assert!((-1.001..=-0.999).contains(&rs), "Sample: {}", rs);

        acr.jump_original(OriginalSamples::new(0), 48_000);

        let output = acr.output(&Info {
            sample_rate: 48_000,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let ls = output[0];
        assert!((0.999..=1.001).contains(&ls), "Sample: {}", ls);
        let rs = output[1];
        assert!((-1.001..=-0.999).contains(&rs), "Sample: {}", rs);
    }

    #[test]
    fn jump_past_end() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey::new(0),
            &test_file_path("48000 16-bit.wav"),
        )
        .unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50, 48_000);

        acr.jump_original(OriginalSamples::new(2_000_000), 48_000);

        assert_eq!(acr.position(), ResampledSamples::new(1_322_978));
    }
}
