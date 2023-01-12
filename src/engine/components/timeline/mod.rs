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

use super::{
    audio_clip::{AudioClip, AudioClipKey},
    track::TrackKey,
};
use crate::engine::{
    traits::{Info, Source},
    utils::{
        key_generator::{self, KeyGenerator},
        remote_push::{RemotePushable, RemotePushedHashMap, RemotePusherHashMap},
        ringbuffer::{self, ringbuffer},
    },
    Sample,
};
pub use timestamp::Timestamp;
pub use track::{TimelineTrack, TimelineTrackKey};

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
    fn new(start: Timestamp, length: Option<TimeStamp>, audio_clip: AudioClip) -> Self {
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

    let (tracks_pusher, tracks_pushed) = HashMap::remote_push();

    let (event_sender, event_receiver) = ringbuffer();

    (
        Timeline {
            max_buffer_size,
            key_generator: KeyGenerator::new(),

            position_updated: position_updated1,
            new_position: new_position1,
            position: position1,

            tracks: tracks_pusher,

            event_sender,
        },
        TimelineProcessor {
            position_updated: position_updated2,
            new_position: new_position2,
            position: position2,

            tracks: tracks_pushed,

            event_receiver,
        },
    )
}

enum Event {
    AddClip {
        track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    },
}

pub struct Timeline {
    max_buffer_size: usize,
    key_generator: KeyGenerator<TimelineTrackKey>,

    position_updated: Arc<AtomicBool>,
    new_position: Arc<AtomicU64>,
    position: Arc<AtomicU64>,

    tracks: RemotePusherHashMap<TimelineTrackKey, TimelineTrack>,

    event_sender: ringbuffer::Sender<Event>,
}
impl Timeline {
    pub fn add_track(
        &mut self,
        output: TrackKey,
    ) -> Result<TimelineTrackKey, TimelineTrackOverflowError> {
        let key = self.key_generator.next()?;

        let timeline_track =
            TimelineTrack::new(output, Arc::clone(&self.position), self.max_buffer_size);
        self.tracks.push((key, timeline_track));
        Ok(key)
    }

    pub fn jump_to(&self, position: u64) {
        self.new_position.store(position, Ordering::SeqCst);
        self.position_updated.store(true, Ordering::SeqCst);
    }

    pub fn remaining_keys(&self) -> u32 {
        self.key_generator.remaining_keys()
    }

    pub fn add_clip(
        &mut self,
        track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<(), AddClipError> {
        // TODO: Check whether clip exists
        if !self.key_generator.in_use(track_key) {
            return Err(AddClipError::InvalidTimelineTrack(track_key));
        }

        self.event_sender.send(Event::AddClip {
            track_key,
            clip_key,
            start,
            length,
        });

        Ok(())
    }
}

pub struct TimelineProcessor {
    position_updated: Arc<AtomicBool>,
    new_position: Arc<AtomicU64>,

    // Only atomic to be Send
    position: Arc<AtomicU64>,

    tracks: RemotePushedHashMap<TimelineTrackKey, TimelineTrack>,

    event_receiver: ringbuffer::Receiver<Event>,
}
impl TimelineProcessor {
    pub fn output(&self, buffer_size: u64) {
        self.position.fetch_add(buffer_size, Ordering::SeqCst);
    }
}
impl TimelineProcessor {
    pub fn poll(&mut self) {
        if self.position_updated.load(Ordering::SeqCst) {
            let new_pos = self.new_position.load(Ordering::SeqCst);
            self.position.store(new_pos, Ordering::SeqCst);
        }

        self.tracks.poll();
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

#[derive(Debug, PartialEq, Eq)]
pub enum AddClipError {
    InvalidTimelineTrack(TimelineTrackKey),
    InvalidClip(AudioClipKey),
}
impl Display for AddClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTimelineTrack(key) => write!(f, "No timeline track with key: {}", key),
            Self::InvalidClip(key) => write!(f, "No audio clip with key: {}", key),
        }
    }
}
impl Error for AddClipError {}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn add_track() {
        let (mut tl, mut tlp) = timeline(10);

        for _ in 0..50 {
            tl.add_track(0).unwrap();
        }

        no_heap! {{
            tlp.poll();

        }}

        assert_eq!(tl.remaining_keys(), u32::MAX - 50);
        assert_eq!(tlp.tracks.len(), 50);
    }

    #[test]
    fn add_clip() {
        let (mut tl, mut tlp) = timeline(10);

        let tk = tl.add_track(0).unwrap();
        tl.add_clip(tk, 0, Timestamp::from_beat_units(0), None)
            .unwrap();

        no_heap! {{
            tlp.poll();

        }}
    }
}
