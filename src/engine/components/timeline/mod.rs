mod audio_clip;
mod timestamp;
mod track;

use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    iter::zip,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use self::audio_clip::AudioClip;

use super::{
    audio_clip_store::{AudioClipStore, ImportError, InvalidAudioClipError},
    stored_audio_clip::{StoredAudioClip, StoredAudioClipKey},
    track::MixerTrackKey,
};
use crate::engine::{
    info::Info,
    utils::{
        dropper::DBox,
        key_generator::{self, KeyGenerator},
        rbtree_node::TreeNode,
        remote_push::{RemotePushable, RemotePushedHashMap, RemotePusherHashMap},
        ringbuffer::{self, ringbuffer},
    },
    Sample, CHANNELS,
};
pub use timestamp::Timestamp;
pub use track::{TimelineTrack, TimelineTrackKey, TimelineTrackState};

pub fn timeline(
    sample_rate: u32,
    bpm_cents: u16,
    max_buffer_size: usize,
) -> (Timeline, TimelineProcessor) {
    let playing1 = Arc::new(AtomicBool::new(false));
    let playing2 = Arc::clone(&playing1);

    let position1 = Arc::new(AtomicU64::new(0));
    let position2 = Arc::clone(&position1);

    let (tracks_pusher, tracks_pushed) = HashMap::remote_push();

    let (event_sender, event_receiver) = ringbuffer();

    (
        Timeline {
            sample_rate,
            bpm_cents,
            key_generator: KeyGenerator::new(),

            playing: playing1,
            position: position1,

            clip_store: AudioClipStore::new(max_buffer_size, sample_rate),
            tracks: tracks_pusher,

            event_sender,
        },
        TimelineProcessor {
            playing: playing2,
            position: position2,
            tracks: tracks_pushed,

            event_receiver,
        },
    )
}

enum Event {
    JumpTo(u64),
    AddClip {
        track_key: MixerTrackKey,
        clip: Box<TreeNode<AudioClip>>,
    },
}

pub struct Timeline {
    sample_rate: u32,
    bpm_cents: u16,
    key_generator: KeyGenerator<TimelineTrackKey>,

    playing: Arc<AtomicBool>,
    /// Should not be mutated from here
    position: Arc<AtomicU64>,

    clip_store: AudioClipStore,
    tracks: RemotePusherHashMap<TimelineTrackKey, DBox<TimelineTrack>>,

    event_sender: ringbuffer::Sender<Event>,
}
impl Timeline {
    pub fn play(&mut self) {
        self.playing.store(true, Ordering::Release);
    }
    pub fn pause(&mut self) {
        self.playing.store(false, Ordering::Release);
    }
    pub fn jump_to(&mut self, position: Timestamp) {
        self.event_sender.send(Event::JumpTo(
            position.samples(self.sample_rate, self.bpm_cents),
        ));
    }
    pub fn playhead_position(&mut self) -> Timestamp {
        Timestamp::from_samples(
            self.position.load(Ordering::Relaxed),
            self.sample_rate,
            self.bpm_cents,
        )
    }

    pub fn import_audio_clip(&mut self, path: &Path) -> Result<StoredAudioClipKey, ImportError> {
        self.clip_store.import(path)
    }
    pub fn stored_audio_clip(
        &self,
        key: StoredAudioClipKey,
    ) -> Result<Arc<StoredAudioClip>, InvalidAudioClipError> {
        self.clip_store.get(key)
    }

    pub fn add_clip(
        &mut self,
        track_key: TimelineTrackKey,
        clip_key: StoredAudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<(), AddClipError> {
        if !self.key_in_use(track_key) {
            return Err(AddClipError::InvalidTimelineTrack(track_key));
        }

        let audio_clip = self
            .clip_store
            .reader(clip_key)
            .or(Err(AddClipError::InvalidClip(clip_key)))?;

        self.event_sender.send(Event::AddClip {
            track_key,
            clip: Box::new(TreeNode::new(AudioClip::new(start, length, audio_clip))),
        });

        Ok(())
    }

    pub fn add_track(
        &mut self,
        output: MixerTrackKey,
    ) -> Result<TimelineTrackKey, TimelineTrackOverflowError> {
        let key = self.key_generator.next()?;

        let timeline_track = TimelineTrack::new(
            output,
            Arc::clone(&self.position),
            self.sample_rate,
            self.bpm_cents,
        );
        self.tracks.push((key, DBox::new(timeline_track)));
        Ok(key)
    }
    pub fn add_tracks<'a>(
        &mut self,
        outputs: Vec<MixerTrackKey>,
    ) -> Result<Vec<TimelineTrackKey>, TimelineTrackOverflowError> {
        let count = outputs.len();

        if self.key_generator.remaining_keys()
            < count.try_into().or(Err(TimelineTrackOverflowError))?
        {
            return Err(TimelineTrackOverflowError);
        }

        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.key_generator.next().expect(
                "next_key() returned error, even though it reported remaining_keys() >= count",
            );
            keys.push(key);
        }

        let tracks = zip(&keys, outputs)
            .map(|(&key, output)| {
                (
                    key,
                    DBox::new(TimelineTrack::new(
                        output,
                        Arc::clone(&self.position),
                        self.sample_rate,
                        self.bpm_cents,
                    )),
                )
            })
            .collect();
        self.tracks.push_multiple(tracks);
        Ok(keys)
    }

    pub fn delete_track(&mut self, key: TimelineTrackKey) -> Result<(), InvalidTimelineTrackError> {
        let result = self.key_generator.free(key);
        if result.is_err() {
            return Err(InvalidTimelineTrackError { key });
        }
        self.tracks.remove(key);
        Ok(())
    }
    pub fn delete_tracks(
        &mut self,
        keys: Vec<TimelineTrackKey>,
    ) -> Result<(), InvalidTimelineTrackError> {
        for &key in &keys {
            if !self.key_in_use(key) {
                return Err(InvalidTimelineTrackError { key });
            }
        }
        for &key in &keys {
            self.key_generator
                .free(key)
                .expect("key_in_use() returned true, even though free() returned error");
        }
        self.tracks.remove_multiple(keys);
        Ok(())
    }

    pub fn reconstruct_track(&mut self, state: &TimelineTrackState, output: MixerTrackKey) {
        let key = state.key;
        self.key_generator
            .reserve(key)
            .expect("Timeline track key already in use");

        let timeline_track = TimelineTrack::new(
            output,
            Arc::clone(&self.position),
            self.sample_rate,
            self.bpm_cents,
        );
        self.tracks.push((key, DBox::new(timeline_track)));
    }

    pub fn reconstruct_tracks<'a>(
        &mut self,
        states: impl Iterator<Item = (&'a TimelineTrackState, MixerTrackKey)>,
    ) {
        let tracks = states.map(|(state, output)| {
            let key = state.key;
            self.key_generator
                .reserve(key)
                .expect("Timeline track key already in use");

            (
                key,
                DBox::new(TimelineTrack::new(
                    output,
                    Arc::clone(&self.position),
                    self.sample_rate,
                    self.bpm_cents,
                )),
            )
        });
        self.tracks.push_multiple(tracks.collect());
    }

    pub fn key_in_use(&self, key: TimelineTrackKey) -> bool {
        self.key_generator.in_use(key)
    }

    pub fn remaining_keys(&self) -> u32 {
        self.key_generator.remaining_keys()
    }

    pub fn track_state(
        &self,
        key: TimelineTrackKey,
    ) -> Result<TimelineTrackState, InvalidTimelineTrackError> {
        if !self.key_in_use(key) {
            return Err(InvalidTimelineTrackError { key });
        }

        Ok(TimelineTrackState { key })
    }
}

pub struct TimelineProcessor {
    playing: Arc<AtomicBool>,
    position: Arc<AtomicU64>,

    tracks: RemotePushedHashMap<TimelineTrackKey, DBox<TimelineTrack>>,

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

    fn add_clip(&mut self, track_key: TimelineTrackKey, timeline_clip: Box<TreeNode<AudioClip>>) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        track.insert_clip(timeline_clip);
    }

    pub fn output(
        &mut self,
        mixer_ins: &mut HashMap<MixerTrackKey, DBox<Vec<Sample>>>,
        info: &Info,
    ) {
        let Info {
            sample_rate: _,
            buffer_size,
        } = *info;

        const NO_BUFFER_MSG: &str = "No buffer found for output track";

        if !self.playing.load(Ordering::Relaxed) {
            for key in self.tracks.keys() {
                let buffer =
                    &mut mixer_ins.get_mut(key).expect(NO_BUFFER_MSG)[..buffer_size * CHANNELS];
                buffer.fill(0.0);
            }
            return;
        }

        for track in self.tracks.values_mut() {
            let key = track.output_track();
            let buffer =
                &mut mixer_ins.get_mut(&key).expect(NO_BUFFER_MSG)[..buffer_size * CHANNELS];
            track.output(info, buffer)
        }
        self.position.fetch_add(
            buffer_size
                .try_into()
                .expect("buffer_size doesn't fit in 64 bits"),
            Ordering::Relaxed,
        );
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
    pub key: TimelineTrackKey,
}
impl Display for InvalidTimelineTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No track with key, {}, on timeline", self.key)
    }
}
impl Error for InvalidTimelineTrackError {}

#[derive(Debug, PartialEq, Eq)]
pub enum AddClipError {
    InvalidTimelineTrack(TimelineTrackKey),
    InvalidClip(StoredAudioClipKey),
}
impl Display for AddClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTimelineTrack(key) => {
                write!(f, "No timeline track with key, {}, on timeline", key)
            }
            Self::InvalidClip(key) => write!(f, "No audio clip with key, {}", key),
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
        let (mut tl, mut tlp) = timeline(40_000, 100_00, 10);

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
        let (mut tl, mut tlp) = timeline(40_000, 100_00, 10);

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
