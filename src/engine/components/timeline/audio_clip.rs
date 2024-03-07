use serde::{Deserialize, Serialize};
use std::cmp::min;

use crate::{
    engine::{
        components::audio_clip_reader::{AudioClipReader, OriginalSamples, ResampledSamples},
        info::Info,
        utils::{key_generator::key_type, min_max, rbtree_node},
        Sample,
    },
    StoredAudioClipKey, Timestamp,
};

// A key for an audio clip, identifying it uniquely across the entire timeline.
key_type!(pub struct AudioClipKey(u32));

// A mirror of `AudioClipProcessor`'s state.
// Does no synchronization.
#[derive(Debug)]
pub struct AudioClip {
    pub key: AudioClipKey,

    /// Start on the timeline
    pub(crate) start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    pub(crate) set_length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    /// This is in the domain of the source clip's sample rate.
    pub(crate) start_offset: OriginalSamples,

    pub(crate) reader: AudioClipReader,
}
impl AudioClip {
    pub fn start(&self) -> Timestamp {
        self.start
    }
    pub fn length(&self, bpm_cents: u16) -> Timestamp {
        self.set_length.unwrap_or(Timestamp::from_samples_ceil(
            self.reader.len_original().into(),
            self.reader.sample_rate_original(),
            bpm_cents,
        ))
    }

    pub fn end(&self, bpm_cents: u16) -> Timestamp {
        self.start + self.length(bpm_cents)
    }

    pub fn overlaps(&self, other: &Self, bpm_cents: u16) -> bool {
        let start1 = self.start;
        let end1 = self.end(bpm_cents);

        let start2 = other.start;
        let end2 = other.end(bpm_cents);

        start1 <= start2 && start2 < end1 || start2 <= start1 && start1 < end2
    }

    pub fn stored_clip(&self) -> StoredAudioClipKey {
        self.reader.key()
    }

    /// Get the data needed to visualize the waveform of the clip.
    /// This will divide the entirety of the clip into `chunks` parts, and for each part it will find the minimum and maximum value for each channel.
    ///
    /// The data will contain the clip's original number of channels, and will be interleaved like this:
    ///
    /// `
    /// [minChannel0Chunk0, maxChannel0Chunk0, minChannel1Chunk0, maxChannel1Chunk0, minChannel0Chunk1, maxChannel0Chunk1, ...]
    /// `
    ///
    /// The data will be normalized to fit within the range of a 16-bit signed integer, such that the highest peak in the clip will be at 32767 or -32767.
    pub fn waveform(&self, chunks: usize) -> Vec<i16> {
        let channels = self.reader.channels_original();
        let mut out = vec![0; 2 * chunks * channels];

        // TODO: Take own start and length into account
        let data = self.reader.output_raw();

        let len: usize = self.reader.len_original().into();
        let chunk_size = len.div_ceil(chunks);
        for (channel_i, channel) in data.iter().enumerate() {
            for (chunk_i, chunk) in channel.chunks(chunk_size).enumerate() {
                let i = (2 * channels * chunk_i) + (2 * channel_i);
                let (min, max) = min_max(chunk.iter().copied(), 0.0);
                out[i] = (min * i16::MAX as f32) as i16;
                out[i + 1] = (max * i16::MAX as f32) as i16;
            }
        }
        out
    }

    pub(crate) fn state(&self) -> AudioClipState {
        AudioClipState {
            key: self.key,
            start_offset: self.start_offset,
            start: self.start,
            length: self.set_length,
            inner: self.reader.key(),
        }
    }
}

#[derive(Debug)]
pub struct AudioClipProcessor {
    /// Start on the timeline
    pub start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    pub length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    /// This is in the domain of the source clip's sample rate.
    pub start_offset: OriginalSamples,

    reader: AudioClipReader,
}
impl AudioClipProcessor {
    pub fn new(
        start: Timestamp,
        length: Option<Timestamp>,
        start_offset: OriginalSamples,
        reader: AudioClipReader,
    ) -> Self {
        AudioClipProcessor {
            start,
            length,
            start_offset,
            reader,
        }
    }

    pub fn end(&self, bpm_cents: u16) -> Timestamp {
        if let Some(length) = self.length {
            self.start + length
        } else {
            self.start
                + Timestamp::from_samples_ceil(
                    (self.reader.len_original() - self.start_offset).into(),
                    self.reader.sample_rate_original(),
                    bpm_cents,
                )
        }
    }

    /// Resets the position to the start of the clip.
    pub fn reset(&mut self, sample_rate: u32) {
        self.reader.jump_original(self.start_offset, sample_rate);
    }

    /// Jumps to the given position relative to the start of the timeline.
    ///
    /// - If the position is before the start of the clip, the position is set to the start of the clip.
    /// - If the position is after the end of the clip, the position is set to the end of the clip.
    pub fn jump(&mut self, pos: Timestamp, sample_rate: u32, bpm_cents: u16) {
        let start_samples = OriginalSamples::new(
            self.start
                .samples(self.reader.sample_rate_original(), bpm_cents),
        );
        let pos_samples =
            OriginalSamples::new(pos.samples(self.reader.sample_rate_original(), bpm_cents));

        // Saturating subtraction means that if the position is before the start of the clip,
        // then the clip is reset to 0.
        let inner_pos = pos_samples.saturating_sub(start_samples) + self.start_offset;

        self.reader.jump_original(inner_pos, sample_rate);
    }

    fn length_samples(&self, sample_rate: u32, bpm_cents: u16) -> ResampledSamples {
        match self.length {
            None => (self.reader.len_original() - self.start_offset)
                .into_resampled(sample_rate, self.reader.sample_rate_original()),
            Some(length) => ResampledSamples::new(length.samples(sample_rate, bpm_cents)),
        }
    }

    /// Outputs to a buffer of at most the requested size (via the info parameter).
    /// If the end is reached the returned buffer is smaller.
    pub fn output(&mut self, bpm_cents: u16, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        let length = self.length_samples(sample_rate, bpm_cents);
        let pos = self.reader.position()
            - self
                .start_offset
                .into_resampled(sample_rate, self.reader.sample_rate_original());
        let remaining = length.saturating_sub(pos);
        let capped_buffer_size = min(buffer_size, remaining.into());

        self.reader.output(&Info {
            sample_rate,
            buffer_size: capped_buffer_size,
        })
    }
}
impl rbtree_node::Keyed for AudioClipProcessor {
    type Key = Timestamp;

    fn key(&self) -> Self::Key {
        self.start
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioClipState {
    pub key: AudioClipKey,
    pub start_offset: OriginalSamples,
    pub start: Timestamp,
    pub length: Option<Timestamp>,
    pub inner: StoredAudioClipKey,
}
