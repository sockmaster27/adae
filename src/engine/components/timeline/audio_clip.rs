use std::cmp::min;

use crate::{
    engine::{
        components::audio_clip_reader::AudioClipReader, info::Info, utils::rbtree_node, Sample,
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
                let remaining = length.samples(sample_rate, bpm_cents) as usize
                    - self.inner.position()
                    + self.start_offset;
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
