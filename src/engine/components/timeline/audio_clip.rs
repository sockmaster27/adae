use serde::{Deserialize, Serialize};
use std::cmp::min;

use crate::{
    engine::{
        components::audio_clip_reader::{AudioClipReader, JumpOutOfBounds},
        info::Info,
        utils::rbtree_node,
        Sample,
    },
    StoredAudioClipKey, Timestamp,
};

pub type AudioClipKey = u32;

/// A mirror of `AudioClipProcessor`'s state.
/// Does no synchronization.
pub struct AudioClip {
    pub key: AudioClipKey,

    /// Start on the timeline
    pub start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    pub length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    pub start_offset: usize,

    pub reader: AudioClipReader,
}
impl AudioClip {
    pub fn end(&self, sample_rate: u32, bpm_cents: u16) -> Timestamp {
        if let Some(length) = self.length {
            self.start + length
        } else {
            self.start
                + Timestamp::from_samples_ceil(
                    self.reader.len(sample_rate) as u64,
                    sample_rate,
                    bpm_cents,
                )
        }
    }

    pub fn overlaps(&self, other: &Self, sample_rate: u32, bpm_cents: u16) -> bool {
        let start1 = self.start;
        let end1 = self.end(sample_rate, bpm_cents);

        let start2 = other.start;
        let end2 = other.end(sample_rate, bpm_cents);

        start1 <= start2 && start2 < end1 || start2 <= start1 && start1 < end2
    }

    pub fn state(&self) -> AudioClipState {
        AudioClipState {
            key: self.key,
            start_offset: self.start_offset,
            start: self.start,
            length: self.length,
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
    pub start_offset: usize,

    reader: AudioClipReader,
}
impl AudioClipProcessor {
    pub fn new(
        start: Timestamp,
        length: Option<Timestamp>,
        start_offset: usize,
        reader: AudioClipReader,
    ) -> Self {
        AudioClipProcessor {
            start,
            length,
            start_offset,
            reader,
        }
    }

    pub fn end(&self, sample_rate: u32, bpm_cents: u16) -> Timestamp {
        if let Some(length) = self.length {
            self.start + length
        } else {
            self.start
                + Timestamp::from_samples_ceil(
                    self.reader.len(sample_rate) as u64,
                    sample_rate,
                    bpm_cents,
                )
        }
    }

    /// Resets the position to the start of the clip.
    pub fn reset(&mut self, sample_rate: u32) {
        self.reader.jump(self.start_offset, sample_rate).unwrap();
    }

    /// Jumps to the given position relative to the start of the timeline.
    pub fn jump_to(
        &mut self,
        pos: Timestamp,
        sample_rate: u32,
        bpm_cents: u16,
    ) -> Result<(), JumpOutOfBounds> {
        let start_samples = self.start.samples(sample_rate, bpm_cents) as usize;
        let pos_samples = pos.samples(sample_rate, bpm_cents) as usize;

        if pos_samples < start_samples + self.start_offset {
            return Err(JumpOutOfBounds);
        }

        self.reader.jump(
            pos_samples - (start_samples + self.start_offset),
            sample_rate,
        )?;

        Ok(())
    }

    fn length_samples(&self, sample_rate: u32, bpm_cents: u16) -> usize {
        match self.length {
            None => self.reader.len(sample_rate),
            Some(length) => length.samples(sample_rate, bpm_cents) as usize,
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
        let pos = self.reader.position();
        let remaining = length - min(pos + self.start_offset, length);
        let capped_buffer_size = min(buffer_size, remaining);

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
    pub start_offset: usize,
    pub start: Timestamp,
    pub length: Option<Timestamp>,
    pub inner: StoredAudioClipKey,
}
