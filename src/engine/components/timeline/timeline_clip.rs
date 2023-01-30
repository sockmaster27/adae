use intrusive_collections::{intrusive_adapter, KeyAdapter, RBTreeLink};

use crate::{
    engine::{
        components::audio_clip::AudioClipReader,
        traits::{Info, Source},
        Sample,
    },
    Timestamp,
};

intrusive_adapter!(pub TimelineClipAdapter = Box<TimelineClip>: TimelineClip { link: RBTreeLink });
impl<'a> KeyAdapter<'a> for TimelineClipAdapter {
    type Key = Timestamp;
    fn get_key(&self, tc: &'a TimelineClip) -> Timestamp {
        tc.start
    }
}

#[derive(Debug)]
pub struct TimelineClip {
    /// Start on the timeline
    pub start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    pub length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    pub start_offset: u64,

    inner: AudioClipReader,

    link: RBTreeLink,
}
impl TimelineClip {
    pub fn new(start: Timestamp, length: Option<Timestamp>, audio_clip: AudioClipReader) -> Self {
        TimelineClip {
            start,
            length,
            start_offset: 0,
            inner: audio_clip,
            link: RBTreeLink::new(),
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

    pub fn output(&mut self, info: &Info) -> &mut [Sample] {
        self.inner.output(info)
    }
}
