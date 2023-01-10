mod timestamp;
mod track;

use symphonia::core::units::TimeStamp;

use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use super::{audio_clip::AudioClip, event_queue::EventReceiver, mixer::TrackKey};
use crate::engine::{
    traits::{Component, Info, Source},
    utils::key_generator::{self, KeyGenerator},
    Sample,
};
pub use timestamp::Timestamp;
use track::timeline_track;
pub use track::{TimelineTrack, TimelineTrackKey, TimelineTrackProcessor};

pub type TimelineClipKey = u32;

#[derive(Debug)]
struct TimelineClip {
    /// Start on the timeline
    start: Timestamp,
    /// Duration on the timeline.
    /// If `None` clip should play till end.
    length: Option<Timestamp>,
    /// Where in the source clip this sound starts.
    /// Relevant if the start has been trimmed off.
    start_offset: u64,

    inner: AudioClip,
}
impl TimelineClip {
    fn new(
        key: TimelineClipKey,
        start: Timestamp,
        length: Option<TimeStamp>,
        audio_clip: AudioClip,
    ) -> Self {
        Self {
            start,
            length: None,
            start_offset: 0,
            inner: audio_clip,
        }
    }

    fn end(&self, sample_rate: u128, bpm: u128) -> Timestamp {
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
                    bpm,
                )
        }
    }

    fn output(&mut self, info: &Info) -> &mut [Sample] {
        self.inner.output(info)
    }
}

pub fn timeline(max_buffer_size: usize) -> (Timeline, TimelineProcessor) {
    let position_updated1 = Arc::new(AtomicBool::new(false));
    let position_updated2 = Arc::clone(&position_updated1);

    let new_position1 = Arc::new(AtomicU64::new(0));
    let new_position2 = Arc::clone(&new_position1);

    let position1 = Arc::new(AtomicU64::new(0));
    let position2 = Arc::clone(&new_position1);

    (
        Timeline {
            max_buffer_size,
            key_generator: KeyGenerator::new(),

            position_updated: position_updated1,
            new_position: new_position1,
            position: position1,

            tracks: HashMap::new(),
        },
        TimelineProcessor {
            position_updated: position_updated2,
            new_position: new_position2,

            position: position2,
        },
    )
}

#[derive(Debug)]
pub struct Timeline {
    max_buffer_size: usize,
    key_generator: KeyGenerator<TimelineTrackKey>,

    position_updated: Arc<AtomicBool>,
    new_position: Arc<AtomicU64>,
    position: Arc<AtomicU64>,

    tracks: HashMap<TimelineTrackKey, TimelineTrack>,
}
impl Timeline {
    pub fn add_track(
        &mut self,
    ) -> Result<(TimelineTrackKey, TimelineTrackProcessor), TimelineTrackOverflowError> {
        let key = self.key_generator.next_key()?;

        let (timeline_track, timeline_track_processor) =
            timeline_track(key, Arc::clone(&self.position), self.max_buffer_size);

        self.tracks.insert(key, timeline_track);

        Ok((key, timeline_track_processor))
    }

    pub fn jump_to(&self, position: u64) {
        self.new_position.store(position, Ordering::SeqCst);
        self.position_updated.store(true, Ordering::SeqCst);
    }

    pub fn remaining_keys(&self) -> u32 {
        self.key_generator.remaining_keys()
    }

    pub fn track(
        &self,
        key: TimelineTrackKey,
    ) -> Result<&TimelineTrack, InvalidTimelineTrackError> {
        self.tracks
            .get(&key)
            .ok_or(InvalidTimelineTrackError { key })
    }
    pub fn track_mut(
        &mut self,
        key: TimelineTrackKey,
    ) -> Result<&mut TimelineTrack, InvalidTimelineTrackError> {
        self.tracks
            .get_mut(&key)
            .ok_or(InvalidTimelineTrackError { key })
    }
}

#[derive(Debug)]
pub struct TimelineProcessor {
    position_updated: Arc<AtomicBool>,
    new_position: Arc<AtomicU64>,

    // Only atomic to be Send
    position: Arc<AtomicU64>,
}
impl TimelineProcessor {
    pub fn output(&self, buffer_size: u64) {
        self.position.fetch_add(buffer_size, Ordering::SeqCst);
    }
}
impl Component for TimelineProcessor {
    fn poll<'a, 'b>(&'a mut self, _: &mut EventReceiver<'a, 'b>) {
        if self.position_updated.load(Ordering::SeqCst) {
            let new_pos = self.new_position.load(Ordering::SeqCst);
            self.position.store(new_pos, Ordering::SeqCst);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TimelineTrackOverflowError;
impl Display for TimelineTrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of tracks has been exceeded. Impressive")
    }
}
impl Error for TimelineTrackOverflowError {}
impl From<key_generator::OverflowError> for TimelineTrackOverflowError {
    fn from(_: key_generator::OverflowError) -> Self {
        Self
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidTimelineTrackError {
    key: TimelineTrackKey,
}
impl Display for InvalidTimelineTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No track with key: {}", self.key)
    }
}
impl Error for InvalidTimelineTrackError {}
