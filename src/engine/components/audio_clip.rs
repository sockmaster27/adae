use std::{
    borrow::Cow,
    cmp::min,
    error::Error,
    fmt::{Debug, Display},
    fs::File,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, Signal},
    codecs::DecoderOptions,
    conv::IntoSample,
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    sample::Sample as SymphoniaSample,
};

use crate::engine::traits::{Info, Source};
use crate::engine::{Sample, CHANNELS};
use crate::zip;

pub type AudioClipKey = u32;

pub struct AudioClipReader {
    inner: Arc<AudioClip>,
    position: usize,
    output_buffer: Vec<Sample>,
}
impl AudioClipReader {
    pub fn new(clip: Arc<AudioClip>, max_buffer_size: usize) -> Self {
        AudioClipReader {
            inner: clip,
            position: 0,
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],
        }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    /// Scales `range` of each channel to the appropriate number of channels,
    /// and loads the interlaced result to `self.output_buffer`.
    fn scale_channels(&mut self, range: Range<usize>) {
        match self.inner.channels() {
            1 => {
                let relevant = &self.inner.data[0][range];
                for (&sample, output_frame) in
                    zip!(relevant, self.output_buffer.chunks_mut(CHANNELS))
                {
                    for output_sample in output_frame {
                        *output_sample = sample;
                    }
                }
            }
            2 => {
                let relevant_left = &self.inner.data[0][range.clone()];
                let relevant_right = &self.inner.data[1][range];
                for ((&left_sample, &right_sample), output_frame) in zip!(
                    relevant_left,
                    relevant_right,
                    self.output_buffer.chunks_mut(CHANNELS)
                ) {
                    output_frame[0] = left_sample;
                    output_frame[1] = right_sample;
                }
            }
            _ => {
                // This should have been caught while importing
                panic!("Clip has incompatible number of channels");
            }
        }
    }

    // Jump to position with sample precision
    pub fn jump(&mut self, position: usize) -> Result<(), JumpOutOfBounds> {
        if position >= self.len() {
            Err(JumpOutOfBounds)
        } else {
            self.position = position;
            Ok(())
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
}
impl Source for AudioClipReader {
    /// Outputs to a buffer of at most the requested size (via the info parameter).
    /// If the end is reached the returned buffer is smaller.
    fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate: _,
            buffer_size,
        } = *info;

        // TODO: Resample

        let remaining = self.inner.len() - self.position;
        let filled = min(remaining, buffer_size);

        let next_position = self.position + filled;
        let relevant_range = self.position..next_position;

        self.position = next_position;

        self.scale_channels(relevant_range);

        &mut self.output_buffer[..filled * CHANNELS]
    }
}
impl Debug for AudioClipReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AudioClipReader {{ inner: {:?}, position(): {}, ... }}",
            self.inner, self.position,
        )
    }
}

#[derive(PartialEq)]
pub struct AudioClip {
    sample_rate: u32,

    /// List of channel buffers
    data: Vec<Vec<Sample>>,
}
impl AudioClip {
    pub fn import(path: &Path) -> Result<Self, ImportError> {
        // Currently the entire clip just gets loaded into memory immediately.
        // I guess that could be improved.

        let file = Box::new(
            File::open(path).or_else(|_| Err(ImportError::FileNotFound(path.to_path_buf())))?,
        );
        let mss = MediaSourceStream::new(file, Default::default());

        let mut hint = Hint::new();
        if let Some(os_extension) = path.extension() {
            if let Some(extension) = os_extension.to_str() {
                hint.with_extension(extension);
            }
        }

        let format_options = FormatOptions::default();
        let metadata_options = MetadataOptions::default();
        let decoder_options = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_options, &metadata_options)
            .or(Err(ImportError::UknownFormat))?;
        let mut format = probed.format;

        let track = format
            .default_track()
            .ok_or_else(|| ImportError::Other("No deafault track".to_owned()))?;
        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_options)
            .or(Err(ImportError::UknownFormat))?;

        let mut sample_rate = 0;
        let mut data = Vec::with_capacity(2);
        let mut first = true;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => Ok(packet),
                Err(SymphoniaError::IoError(_)) => break,
                Err(e) => Err(ImportError::Other(format!("{}", e))),
            }?;
            if packet.track_id() != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(received_buffer) => {
                    if first {
                        first = false;

                        let channels = received_buffer.spec().channels.count();
                        sample_rate = received_buffer.spec().rate;

                        // TODO: Support more than 2 channels
                        if channels > 2 {
                            return Err(ImportError::TooManyChannels);
                        }

                        for _ in 0..channels {
                            data.push(Vec::new());
                        }
                    }

                    Self::extend_from_buffer(&mut data, received_buffer);
                }
                Err(e) => panic!("{}", e),
            }
        }

        Ok(Self { sample_rate, data })
    }
    fn extend_from_buffer(data: &mut Vec<Vec<Sample>>, buffer_ref: AudioBufferRef) {
        // Bruh
        use AudioBufferRef as A;
        match buffer_ref {
            A::U8(buffer) => extend(data, buffer),
            A::S8(buffer) => extend(data, buffer),
            A::U16(buffer) => extend(data, buffer),
            A::U24(buffer) => extend(data, buffer),
            A::U32(buffer) => extend(data, buffer),
            A::S16(buffer) => extend(data, buffer),
            A::S24(buffer) => extend(data, buffer),
            A::S32(buffer) => extend(data, buffer),
            A::F32(buffer) => extend(data, buffer),
            A::F64(buffer) => extend(data, buffer),
        };

        fn extend<S>(data: &mut Vec<Vec<Sample>>, buffer: Cow<AudioBuffer<S>>)
        where
            S: SymphoniaSample + IntoSample<Sample>,
        {
            for (chan_i, output) in data.iter_mut().enumerate() {
                let received = buffer.chan(chan_i);
                for &sample in received {
                    output.push(sample.into_sample());
                }
            }
        }
    }

    // Number of channels
    pub fn channels(&self) -> usize {
        self.data.len()
    }

    /// Number of frames in total / Number of samples per channel
    pub fn len(&self) -> usize {
        // This should be equal across all channels
        self.data[0].len()
    }
}
impl Debug for AudioClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AudioClip {{ sample_rate: {}, ..., channels(): {}, len(): {} }}",
            self.sample_rate,
            self.channels(),
            self.len(),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    FileNotFound(PathBuf),
    UknownFormat,
    TooManyChannels,
    Other(String),
}
impl Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::FileNotFound(test_file_path) => {
                format!(
                    "File could not be found: {}",
                    test_file_path.to_string_lossy()
                )
            }
            Self::UknownFormat => "File format not supported".to_owned(),
            Self::TooManyChannels => {
                "Files with more than 2 channels are not currently supported".to_owned()
            }
            Self::Other(msg) => {
                format!("File could not be imported. Failed with error: {}", msg)
            }
        };
        f.write_str(&msg)
    }
}
impl Error for ImportError {}

#[derive(Debug, PartialEq, Eq)]
pub struct JumpOutOfBounds;
impl Display for JumpOutOfBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Attempted to jump past end of audio clip data")
    }
}
impl Error for JumpOutOfBounds {}

#[cfg(test)]
mod tests {
    use crate::engine::utils::test_file_path;

    use super::*;

    fn test_lossless(ac: AudioClip) {
        assert_eq!(ac.channels(), 2);
        assert_eq!(ac.sample_rate, 44100);

        assert_eq!(ac.len(), 1_322_978);

        // These should be 1.0 and -1.0 exactly, but sample conversion skews that a little bit
        let first_left_sample = ac.data[0][0];
        assert!(0.999 <= first_left_sample && first_left_sample <= 1.001);
        let first_right_sample = ac.data[1][0];
        assert!(-1.001 <= first_right_sample && first_right_sample <= -0.999);
    }
    fn test_lossy(ac: AudioClip) {
        assert_eq!(ac.channels(), 2);
        assert_eq!(ac.sample_rate, 44100);

        // Lossy encoding might introduce some extra samples in the beginning and end
        assert!(ac.len() >= 1_322_978);
        assert!(ac.len() < 1_330_000);
    }

    #[test]
    fn import_wav_44100_16_bit() {
        let ac = AudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        test_lossless(ac);
    }
    #[test]
    fn import_wav_44100_24_bit() {
        let ac = AudioClip::import(&test_file_path("44100 24-bit.wav")).unwrap();
        test_lossless(ac);
    }
    #[test]
    fn import_wav_44100_32_float() {
        let ac = AudioClip::import(&test_file_path("44100 32-float.wav")).unwrap();
        test_lossless(ac);
    }

    #[test]
    fn import_flac_4410_l5_16_bit() {
        let ac = AudioClip::import(&test_file_path("44100 L5 16-bit.flac")).unwrap();
        test_lossless(ac);
    }

    #[test]
    fn import_mp3_44100_joint_stereo() {
        let ac = AudioClip::import(&test_file_path(
            "44100 preset-standard-fast joint-stereo.mp3",
        ))
        .unwrap();
        test_lossy(ac);
    }
    #[test]
    fn import_mp3_44100_stereo() {
        let ac =
            AudioClip::import(&test_file_path("44100 preset-standard-fast stereo.mp3")).unwrap();
        test_lossy(ac);
    }

    #[test]
    fn import_ogg_44100_q5() {
        let ac = AudioClip::import(&test_file_path("44100 Q5.ogg")).unwrap();
        test_lossy(ac);
    }

    #[test]
    fn bad_test_file_path() {
        let test_file_path = test_file_path("lorem ipsum");
        let result = AudioClip::import(&test_file_path);
        assert_eq!(result, Err(ImportError::FileNotFound(test_file_path)));
    }
    #[test]
    fn unsupported_file() {
        let result = AudioClip::import(&test_file_path("44100 Q160 [unsupported].m4a"));
        assert_eq!(result, Err(ImportError::UknownFormat));
    }

    #[test]
    fn reader_output() {
        let ac = AudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50);

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let first_left_sample = output[0];
        assert!(0.999 <= first_left_sample && first_left_sample <= 1.001);
        let first_right_sample = output[1];
        assert!(-1.001 <= first_right_sample && first_right_sample <= -0.999);

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let first_left_sample = output[0];
        assert!(-1.001 <= first_left_sample && first_left_sample <= -0.999);
        let first_right_sample = output[1];
        assert!(0.999 <= first_right_sample && first_right_sample <= 1.001);
    }

    #[test]
    fn reader_output_past_end() {
        let ac = AudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 1_322_978 + 10);

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 1_322_978 + 10,
        });

        assert_eq!(output.len(), 1_322_978 * CHANNELS);

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 10,
        });

        assert_eq!(output.len(), 0);
    }

    #[test]
    fn reader_jump() {
        let ac = AudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50);

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let first_left_sample = output[0];
        assert!(0.999 <= first_left_sample && first_left_sample <= 1.001);
        let first_right_sample = output[1];
        assert!(-1.001 <= first_right_sample && first_right_sample <= -0.999);

        acr.jump(0).unwrap();

        let output = acr.output(&Info {
            sample_rate: 44100,
            buffer_size: 50,
        });

        assert_eq!(output.len(), 50 * CHANNELS);

        let first_left_sample = output[0];
        assert!(0.999 <= first_left_sample && first_left_sample <= 1.001);
        let first_right_sample = output[1];
        assert!(-1.001 <= first_right_sample && first_right_sample <= -0.999);
    }

    #[test]
    fn reader_jump_out_of_bounds() {
        let ac = AudioClip::import(&test_file_path("44100 16-bit.wav")).unwrap();
        let mut acr = AudioClipReader::new(Arc::new(ac), 50);

        assert_eq!(acr.jump(1_322_978), Err(JumpOutOfBounds));
    }
}
