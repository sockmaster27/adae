mod timeline_clip;
mod timestamp;
mod track;

use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use self::timeline_clip::TimelineClip;

use super::{
    audio_clip::AudioClipKey,
    audio_clip_store::{AudioClipStore, ImportError},
    track::TrackKey,
};
use crate::engine::utils::{
    key_generator::{self, KeyGenerator},
    remote_push::{RemotePushable, RemotePushedHashMap, RemotePusherHashMap},
    ringbuffer::{self, ringbuffer},
};
pub use timestamp::Timestamp;
pub use track::{TimelineTrack, TimelineTrackKey};

pub fn timeline(sample_rate: u32, max_buffer_size: usize) -> (Timeline, TimelineProcessor) {
    let position1 = Arc::new(AtomicU64::new(0));
    let position2 = Arc::clone(&position1);

    let (tracks_pusher, tracks_pushed) = HashMap::remote_push();

    let (event_sender, event_receiver) = ringbuffer();

    (
        Timeline {
            max_buffer_size,
            sample_rate,
            key_generator: KeyGenerator::new(),

            position: position1,

            clip_store: AudioClipStore::new(max_buffer_size),
            tracks: tracks_pusher,

            event_sender,
        },
        TimelineProcessor {
            position: position2,
            bpm_cents: 120,
            tracks: tracks_pushed,

            event_receiver,
        },
    )
}

enum Event {
    JumpTo(u64),
    AddClip {
        track_key: TrackKey,
        clip: Box<TimelineClip>,
    },
}

pub struct Timeline {
    max_buffer_size: usize,
    sample_rate: u32,
    key_generator: KeyGenerator<TimelineTrackKey>,

    /// Should not be mutated from here
    position: Arc<AtomicU64>,

    clip_store: AudioClipStore,
    tracks: RemotePusherHashMap<TimelineTrackKey, TimelineTrack>,

    event_sender: ringbuffer::Sender<Event>,
}
impl Timeline {
    pub fn import_audio_clip(&mut self, path: &Path) -> Result<AudioClipKey, ImportError> {
        self.clip_store.import(path)
    }

    pub fn add_track(
        &mut self,
        output: TrackKey,
    ) -> Result<TimelineTrackKey, TimelineTrackOverflowError> {
        let key = self.key_generator.next()?;

        let timeline_track = TimelineTrack::new(
            output,
            Arc::clone(&self.position),
            self.sample_rate,
            self.max_buffer_size,
        );
        self.tracks.push((key, timeline_track));
        Ok(key)
    }

    pub fn key_in_use(&self, key: TimelineTrackKey) -> bool {
        self.key_generator.in_use(key)
    }

    pub fn jump_to(&mut self, position: u64) {
        self.event_sender.send(Event::JumpTo(position))
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
        if !self.key_in_use(track_key) {
            return Err(AddClipError::InvalidTimelineTrack(track_key));
        }

        let audio_clip = self
            .clip_store
            .get(clip_key)
            .ok_or(AddClipError::InvalidClip(clip_key))?;

        self.event_sender.send(Event::AddClip {
            track_key,
            clip: Box::new(TimelineClip::new(start, length, audio_clip)),
        });

        Ok(())
    }
}

pub struct TimelineProcessor {
    // Only atomic to be Send.
    // Could be Rc<Cell<u64>> if tracks was a regular HashMap.
    position: Arc<AtomicU64>,

    bpm_cents: u16,
    tracks: RemotePushedHashMap<TimelineTrackKey, TimelineTrack>,

    event_receiver: ringbuffer::Receiver<Event>,
}
impl TimelineProcessor {
    pub fn poll(&mut self) {
        self.tracks.poll();

        for _ in 0..256 {
            let event_option = self.event_receiver.recv();
            match event_option {
                None => break,

                Some(event) => match event {
                    Event::JumpTo(pos) => self.position.store(pos, Ordering::Relaxed),
                    Event::AddClip { track_key, clip } => self.add_clip(track_key, clip),
                },
            }
        }
    }

    pub fn output(&self, buffer_size: u64) {
        self.position.fetch_add(buffer_size, Ordering::Relaxed);
    }

    fn add_clip(&mut self, track_key: TimelineTrackKey, timeline_clip: Box<TimelineClip>) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        track.insert_clip(timeline_clip, self.bpm_cents);
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
    use crate::engine::utils::test_file_path;

    use super::*;

    #[test]
    fn add_track() {
        let (mut tl, mut tlp) = timeline(40_000, 10);

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
        let (mut tl, mut tlp) = timeline(40_000, 10);

        let ck = tl
            .import_audio_clip(&test_file_path("44100 16-bit.wav"))
            .unwrap();
        let tk = tl.add_track(0).unwrap();
        tl.add_clip(tk, ck, Timestamp::from_beat_units(0), None)
            .unwrap();

        no_heap! {{
            tlp.poll();

        }}
    }
}
