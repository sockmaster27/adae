use std::cmp::min;

use crate::{
    engine::{
        components::audio_clip_reader::{AudioClipReader, JumpOutOfBounds},
        info::Info,
        utils::rbtree_node,
        Sample,
    },
    Timestamp,
};

#[derive(Debug)]
pub struct AudioClip {
    /// Start on the timeline
    pub start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    pub length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    pub start_offset: usize,

    inner: AudioClipReader,
}
impl AudioClip {
    pub fn new(start: Timestamp, length: Option<Timestamp>, reader: AudioClipReader) -> Self {
        AudioClip {
            start,
            length,
            start_offset: 0,
            inner: reader,
        }
    }

    pub fn end(&self, sample_rate: u32, bpm_cents: u16) -> Timestamp {
        if let Some(length) = self.length {
            self.start + length
        } else {
            self.start
                + Timestamp::from_samples(
                    self.inner
                        .len()
                        .try_into()
                        .expect("Length of audio clip too long"),
                    sample_rate,
                    bpm_cents,
                )
        }
    }

    /// Resets the position to the start of the clip.
    pub fn reset(&mut self, sample_rate: u32, max_buffer_size: usize) {
        self.inner
            .jump(self.start_offset, sample_rate, max_buffer_size)
            .unwrap();
    }

    /// Jumps to the given position relative to the start of the timeline.
    pub fn jump_to(
        &mut self,
        pos: Timestamp,
        sample_rate: u32,
        bpm_cents: u16,
        max_buffer_size: usize,
    ) -> Result<(), JumpOutOfBounds> {
        let start_samples = self.start.samples(sample_rate, bpm_cents) as usize;
        let pos_samples = pos.samples(sample_rate, bpm_cents) as usize;

        if pos_samples < start_samples + self.start_offset {
            return Err(JumpOutOfBounds);
        }

        self.inner.jump(
            pos_samples - (start_samples + self.start_offset),
            sample_rate,
            max_buffer_size,
        )?;

        Ok(())
    }

    /// Outputs to a buffer of at most the requested size (via the info parameter).
    /// If the end is reached the returned buffer is smaller.
    pub fn output(&mut self, bpm_cents: u16, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        let capped_buffer_size = match self.length {
            None => buffer_size,
            Some(length) => {
                let length = length.samples(sample_rate, bpm_cents) as usize;
                let pos = self.inner.position(sample_rate);
                if pos + self.start_offset >= length {
                    return &mut [];
                }
                let remaining = length - (pos + self.start_offset);
                min(buffer_size, remaining)
            }
        };

        self.inner.output(&Info {
            sample_rate,
            buffer_size: capped_buffer_size,
        })
    }
}
impl rbtree_node::Keyed for AudioClip {
    type Key = Timestamp;

    fn key(&self) -> Self::Key {
        self.start
    }
}
