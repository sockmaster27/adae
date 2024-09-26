use std::{
    borrow::Cow,
    error::Error,
    fmt::{Debug, Display},
    fs::File,
    path::{Path, PathBuf},
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

use crate::engine::{
    utils::{key_generator::key_type, min_max},
    Sample,
};

key_type!(pub struct StoredAudioClipKey(u32));

/// Number of samples per chunk in the waveform data.
pub const SAMPLES_PER_WAVEFORM_CHUNK: usize = 1024;

/// An audio clip that has been imported.
#[derive(PartialEq)]
pub struct StoredAudioClip {
    key: StoredAudioClipKey,

    waveform_data: Vec<i16>,

    sample_rate: u32,
    /// List of channel buffers
    audio_data: Vec<Vec<Sample>>,
}
impl StoredAudioClip {
    pub fn import(key: StoredAudioClipKey, path: &Path) -> Result<Self, ImportError> {
        // Currently the entire clip just gets loaded into memory immediately.
        // I guess that could be improved.

        let file =
            Box::new(File::open(path).map_err(|_| ImportError::FileNotFound(path.to_path_buf()))?);
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
        let mut audio_data = Vec::with_capacity(2);
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
                            audio_data.push(Vec::new());
                        }
                    }

                    Self::extend_from_buffer(&mut audio_data, received_buffer);
                }
                Err(e) => panic!("{}", e),
            }
        }

        let channels = audio_data.len();
        let len = audio_data[0].len();

        let chunks = len / SAMPLES_PER_WAVEFORM_CHUNK;
        let mut waveform_data = vec![0; 2 * chunks * channels];
        let chunk_size = len.div_ceil(chunks);
        for (channel_i, channel) in audio_data.iter().enumerate() {
            for (chunk_i, chunk) in channel.chunks(chunk_size).enumerate() {
                let i = (2 * channels * chunk_i) + (2 * channel_i);
                let (min, max) = min_max(chunk.iter().copied(), 0.0);
                waveform_data[i] = (min * i16::MAX as f32) as i16;
                waveform_data[i + 1] = (max * i16::MAX as f32) as i16;
            }
        }

        Ok(Self {
            key,
            waveform_data,
            sample_rate,
            audio_data,
        })
    }
    fn extend_from_buffer(data: &mut [Vec<Sample>], buffer_ref: AudioBufferRef) {
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

        fn extend<S>(data: &mut [Vec<Sample>], buffer: Cow<AudioBuffer<S>>)
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

    pub fn key(&self) -> StoredAudioClipKey {
        self.key
    }

    pub fn waveform_data(&self) -> &[i16] {
        &self.waveform_data
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn audio_data(&self) -> &[Vec<Sample>] {
        &self.audio_data
    }

    /// Number of channels
    pub fn channels(&self) -> usize {
        self.audio_data.len()
    }

    /// Number of frames (samples per channel) in total
    pub fn length(&self) -> usize {
        // All channels should have the same length
        debug_assert!(self
            .audio_data
            .iter()
            .all(|channel| channel.len() == self.audio_data[0].len()));

        self.audio_data[0].len()
    }
}
impl Debug for StoredAudioClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredAudioClip")
            .field("key", &self.key)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels())
            .field("length", &self.length())
            .finish_non_exhaustive()
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

#[cfg(test)]
mod tests {
    use crate::engine::utils::test_file_path;

    use super::*;

    fn test_lossless(ac: StoredAudioClip, sample_rate: u32) {
        assert_eq!(ac.channels(), 2);
        assert_eq!(ac.sample_rate, sample_rate);

        assert_eq!(ac.length(), 1_322_978);

        // These should be 1.0 and -1.0 exactly, but sample conversion skews that a little bit
        let first_left_sample = ac.audio_data[0][0];
        assert!((0.999..=1.001).contains(&first_left_sample));
        let first_right_sample = ac.audio_data[1][0];
        assert!((-1.001..=-0.999).contains(&first_right_sample));
    }
    fn test_lossy(ac: StoredAudioClip, sample_rate: u32) {
        assert_eq!(ac.channels(), 2);
        assert_eq!(ac.sample_rate, sample_rate);

        // Lossy encoding might introduce some extra samples in the beginning and end
        assert!(ac.length() >= 1_322_978);
        assert!(ac.length() < 1_330_000);
    }

    #[test]
    fn import_wav_22050_16_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("22050 16-bit.wav"))
                .unwrap();
        test_lossless(ac, 22050);
    }
    #[test]
    fn import_wav_22050_24_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("22050 24-bit.wav"))
                .unwrap();
        test_lossless(ac, 22050);
    }
    #[test]
    fn import_wav_22050_32_float() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("22050 32-float.wav"))
                .unwrap();
        test_lossless(ac, 22050);
    }

    #[test]
    fn import_flac_22050_l5_16_bit() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("22050 L5 16-bit.flac"),
        )
        .unwrap();
        test_lossless(ac, 22050);
    }

    #[test]
    fn import_mp3_22050_joint_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("22050 preset-standard joint-stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 22050);
    }
    #[test]
    fn import_mp3_22050_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("22050 preset-standard stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 22050);
    }

    #[test]
    fn import_ogg_22050_q5() {
        let ac = StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("22050 Q5.ogg"))
            .unwrap();
        test_lossy(ac, 22050);
    }

    #[test]
    fn import_wav_44100_16_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("44100 16-bit.wav"))
                .unwrap();
        test_lossless(ac, 44100);
    }
    #[test]
    fn import_wav_44100_24_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("44100 24-bit.wav"))
                .unwrap();
        test_lossless(ac, 44100);
    }
    #[test]
    fn import_wav_44100_32_float() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("44100 32-float.wav"))
                .unwrap();
        test_lossless(ac, 44100);
    }

    #[test]
    fn import_flac_44100_l5_16_bit() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("44100 L5 16-bit.flac"),
        )
        .unwrap();
        test_lossless(ac, 44100);
    }

    #[test]
    fn import_mp3_44100_joint_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("44100 preset-standard joint-stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 44100);
    }
    #[test]
    fn import_mp3_44100_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("44100 preset-standard stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 44100);
    }

    #[test]
    fn import_ogg_44100_q5() {
        let ac = StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("44100 Q5.ogg"))
            .unwrap();
        test_lossy(ac, 44100);
    }

    #[test]
    fn import_wav_48000_16_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("48000 16-bit.wav"))
                .unwrap();
        test_lossless(ac, 48000);
    }
    #[test]
    fn import_wav_48000_24_bit() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("48000 24-bit.wav"))
                .unwrap();
        test_lossless(ac, 48000);
    }
    #[test]
    fn import_wav_48000_32_float() {
        let ac =
            StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("48000 32-float.wav"))
                .unwrap();
        test_lossless(ac, 48000);
    }

    #[test]
    fn import_flac_48000_l5_16_bit() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("48000 L5 16-bit.flac"),
        )
        .unwrap();
        test_lossless(ac, 48000);
    }

    #[test]
    fn import_mp3_48000_joint_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("48000 preset-standard joint-stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 48000);
    }
    #[test]
    fn import_mp3_48000_stereo() {
        let ac = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("48000 preset-standard stereo.mp3"),
        )
        .unwrap();
        test_lossy(ac, 48000);
    }

    #[test]
    fn import_ogg_48000_q5() {
        let ac = StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path("48000 Q5.ogg"))
            .unwrap();
        test_lossy(ac, 48000);
    }

    #[test]
    fn bad_test_file_path() {
        let test_file_path = test_file_path("lorem ipsum");
        let result = StoredAudioClip::import(StoredAudioClipKey(0), &test_file_path);
        assert_eq!(result, Err(ImportError::FileNotFound(test_file_path)));
    }
    #[test]
    fn unsupported_file() {
        let result = StoredAudioClip::import(
            StoredAudioClipKey(0),
            &test_file_path("44100 Q160 [unsupported].m4a"),
        );
        assert_eq!(result, Err(ImportError::UknownFormat));
    }
}
