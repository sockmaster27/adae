mod audio_clip;
mod timestamp;
mod track;

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{Debug, Display},
    iter::zip,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use super::{
    audio_clip_store::{
        AudioClipStore, AudioClipStoreState, ImportError, InvalidStoredAudioClipError,
    },
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
use audio_clip::AudioClipProcessor;
pub use audio_clip::{AudioClip, AudioClipKey, AudioClipState};
pub use timestamp::Timestamp;
use track::TimelineTrack;
pub use track::{TimelineTrackKey, TimelineTrackProcessor, TimelineTrackState};

pub fn timeline(
    state: &TimelineState,
    sample_rate: u32,
    max_buffer_size: usize,
) -> (Timeline, TimelineProcessor, Vec<ImportError>) {
    let TimelineState {
        bpm_cents,
        audio_clip_store: store_state,
        tracks: track_states,
    } = state;

    let playing1 = Arc::new(AtomicBool::new(false));
    let playing2 = Arc::clone(&playing1);

    let position = Arc::new(AtomicU64::new(0));

    let (clip_store, import_errors) =
        AudioClipStore::new(store_state, sample_rate, max_buffer_size);

    let tracks = HashMap::from_iter(track_states.iter().map(|state| {
        (
            state.key,
            TimelineTrack {
                output_track: state.output_track,
                clips: HashMap::from_iter(state.clips.iter().map(|state| {
                    (
                        state.key,
                        AudioClip {
                            key: state.key,
                            start: state.start,
                            length: state.length,
                            start_offset: state.start_offset,
                            reader: clip_store
                                .reader(state.inner)
                                .expect("An invalid audio clip was referenced"),
                        },
                    )
                })),
            },
        )
    }));

    let (tracks_pusher, tracks_pushed) = HashMap::from_iter(track_states.iter().map(|state| {
        let mut track = TimelineTrackProcessor::new(
            state.output_track,
            Arc::clone(&position),
            sample_rate,
            *bpm_cents,
        );

        for state in state.clips.iter() {
            track.insert_clip(Box::new(TreeNode::new(AudioClipProcessor::new(
                state.start,
                state.length,
                state.start_offset,
                clip_store
                    .reader(state.inner)
                    .expect("An invalid audio clip was referenced"),
            ))));
        }
        (state.key, DBox::new(track))
    }))
    .into_remote_push();

    let track_key_generator = KeyGenerator::from_iter(tracks.keys().copied());
    let clip_key_generator = KeyGenerator::from_iter(
        tracks
            .values()
            .flat_map(|track| track.clips.keys().copied()),
    );

    let (event_sender, event_receiver) = ringbuffer();

    (
        Timeline {
            sample_rate,
            bpm_cents: *bpm_cents,
            track_key_generator,
            clip_key_generator,

            playing: playing1,
            position: Arc::clone(&position),

            clip_store,
            tracks,
            track_processors: tracks_pusher,

            event_sender,
        },
        TimelineProcessor {
            sample_rate,
            bpm_cents: *bpm_cents,

            playing: playing2,
            position,
            tracks: tracks_pushed,

            event_receiver,
        },
        import_errors,
    )
}

enum Event {
    JumpTo(Timestamp),
    AddClip {
        track_key: MixerTrackKey,
        clip: Box<TreeNode<AudioClipProcessor>>,
    },
    AddClips {
        track_key: MixerTrackKey,
        #[allow(clippy::vec_box)]
        clips: DBox<Vec<Box<TreeNode<AudioClipProcessor>>>>,
    },
    DeleteClip {
        track_key: MixerTrackKey,
        clip_start: Timestamp,
    },
    DeleteClips {
        track_key: MixerTrackKey,
        clip_starts: DBox<Vec<Timestamp>>,
    },
}

pub struct Timeline {
    sample_rate: u32,
    bpm_cents: u16,
    track_key_generator: KeyGenerator<TimelineTrackKey>,
    clip_key_generator: KeyGenerator<AudioClipKey>,

    playing: Arc<AtomicBool>,
    /// Should not be mutated from here
    position: Arc<AtomicU64>,

    clip_store: AudioClipStore,
    tracks: HashMap<TimelineTrackKey, TimelineTrack>,
    track_processors: RemotePusherHashMap<TimelineTrackKey, DBox<TimelineTrackProcessor>>,

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
        self.event_sender.send(Event::JumpTo(position));
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
    ) -> Result<Arc<StoredAudioClip>, InvalidStoredAudioClipError> {
        self.clip_store.get(key)
    }

    pub fn stored_audio_clips(&self) -> impl Iterator<Item = Arc<StoredAudioClip>> + '_ {
        self.clip_store.iter()
    }

    fn add_audio_clip_inner(
        &mut self,
        track_key: TimelineTrackKey,
        clip_state: AudioClipState,
    ) -> Result<(), AddClipError> {
        if !self.key_in_use(track_key) {
            return Err(AddClipError::InvalidTimelineTrack(track_key));
        }

        let AudioClipState {
            key: clip_key,
            start_offset,
            start,
            length,
            inner: stored_clip_key,
        } = clip_state;

        let reader1 = self
            .clip_store
            .reader(stored_clip_key)
            .or(Err(AddClipError::InvalidClip(stored_clip_key)))?;
        let audio_clip = AudioClip {
            key: clip_key,
            start,
            length,
            start_offset,
            reader: reader1,
        };

        let reader2 = self.clip_store.reader(stored_clip_key).unwrap();
        let audio_clip_processor = AudioClipProcessor::new(start, length, 0, reader2);

        let track = self.tracks.get_mut(&track_key).unwrap();
        for clip in track.clips.values() {
            if clip.overlaps(&audio_clip, self.sample_rate, self.bpm_cents) {
                return Err(AddClipError::Overlapping);
            }
        }

        track.clips.insert(clip_key, audio_clip);

        self.event_sender.send(Event::AddClip {
            track_key,
            clip: Box::new(TreeNode::new(audio_clip_processor)),
        });

        Ok(())
    }

    fn add_audio_clips_inner(
        &mut self,
        track_key: TimelineTrackKey,
        clip_states: &[AudioClipState],
    ) -> Result<(), AddClipError> {
        if !self.key_in_use(track_key) {
            return Err(AddClipError::InvalidTimelineTrack(track_key));
        }

        let clip_processors = clip_states
            .iter()
            .map(|clip_state| {
                let AudioClipState {
                    key: clip_key,
                    start_offset,
                    start,
                    length,
                    inner: stored_clip_key,
                } = *clip_state;

                let reader1 = self
                    .clip_store
                    .reader(stored_clip_key)
                    .or(Err(AddClipError::InvalidClip(stored_clip_key)))?;
                let audio_clip = AudioClip {
                    key: clip_key,
                    start,
                    length,
                    start_offset,
                    reader: reader1,
                };

                let reader2 = self.clip_store.reader(stored_clip_key).unwrap();
                let audio_clip_processor = AudioClipProcessor::new(start, length, 0, reader2);

                let track = self.tracks.get_mut(&track_key).unwrap();
                for clip in track.clips.values() {
                    if clip.overlaps(&audio_clip, self.sample_rate, self.bpm_cents) {
                        return Err(AddClipError::Overlapping);
                    }
                }

                track.clips.insert(clip_key, audio_clip);

                Ok(Box::new(TreeNode::new(audio_clip_processor)))
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.event_sender.send(Event::AddClips {
            track_key,
            clips: DBox::new(clip_processors),
        });

        Ok(())
    }

    pub fn add_audio_clip(
        &mut self,
        track_key: TimelineTrackKey,
        stored_clip_key: StoredAudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<AudioClipKey, AddClipError> {
        let key = self.clip_key_generator.peek_next().unwrap();
        self.add_audio_clip_inner(
            track_key,
            AudioClipState {
                key,
                start_offset: 0,
                start,
                length,
                inner: stored_clip_key,
            },
        )?;
        self.clip_key_generator.reserve(key).unwrap();
        Ok(key)
    }

    pub fn audio_clip(
        &self,
        track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
    ) -> Result<&AudioClip, InvalidAudioClipError> {
        let track = self
            .tracks
            .get(&track_key)
            .ok_or(InvalidAudioClipError::InvalidTrack { track_key })?;
        track
            .clips
            .get(&clip_key)
            .ok_or(InvalidAudioClipError::InvalidClip {
                track_key,
                clip_key,
            })
    }

    pub fn audio_clips(
        &self,
        track_key: TimelineTrackKey,
    ) -> Result<impl Iterator<Item = &AudioClip> + '_, InvalidTimelineTrackError> {
        let track = self
            .tracks
            .get(&track_key)
            .ok_or(InvalidTimelineTrackError { key: track_key })?;
        Ok(track.clips.values())
    }

    pub fn delete_audio_clip(
        &mut self,
        track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
    ) -> Result<(), InvalidAudioClipError> {
        let track = self
            .tracks
            .get_mut(&track_key)
            .ok_or(InvalidAudioClipError::InvalidTrack { track_key })?;
        let clip = track
            .clips
            .remove(&clip_key)
            .ok_or(InvalidAudioClipError::InvalidClip {
                track_key,
                clip_key,
            })?;

        self.clip_key_generator
            .free(clip_key)
            .expect("Clip key already freed");

        self.event_sender.send(Event::DeleteClip {
            track_key,
            clip_start: clip.start,
        });

        Ok(())
    }
    pub fn delete_audio_clips(
        &mut self,
        track_key: TimelineTrackKey,
        clip_keys: Vec<AudioClipKey>,
    ) -> Result<(), InvalidAudioClipError> {
        let track = self
            .tracks
            .get_mut(&track_key)
            .ok_or(InvalidAudioClipError::InvalidTrack { track_key })?;

        let clips = clip_keys
            .iter()
            .map(|&clip_key| {
                track
                    .clips
                    .remove(&clip_key)
                    .ok_or(InvalidAudioClipError::InvalidClip {
                        track_key,
                        clip_key,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        for &clip_key in &clip_keys {
            track.clips.remove(&clip_key);
            self.clip_key_generator
                .free(clip_key)
                .expect("Clip key already freed");
        }

        self.event_sender.send(Event::DeleteClips {
            track_key,
            clip_starts: DBox::new(clips.iter().map(|clip| clip.start).collect()),
        });

        Ok(())
    }

    pub fn reconstruct_audio_clip(
        &mut self,
        track_key: TimelineTrackKey,
        clip_state: AudioClipState,
    ) -> Result<AudioClipKey, AudioClipReconstructionError> {
        let key = clip_state.key;
        if self.clip_key_generator.in_use(key) {
            return Err(AudioClipReconstructionError::KeyInUse(key));
        }
        self.add_audio_clip_inner(track_key, clip_state)?;
        self.clip_key_generator.reserve(key).unwrap();
        Ok(key)
    }
    pub fn reconstruct_audio_clips(
        &mut self,
        track_key: TimelineTrackKey,
        clip_states: Vec<AudioClipState>,
    ) -> Result<Vec<AudioClipKey>, AudioClipReconstructionError> {
        let keys = clip_states.iter().map(|c| c.key).collect();
        for &key in &keys {
            if self.clip_key_generator.in_use(key) {
                return Err(AudioClipReconstructionError::KeyInUse(key));
            }
        }
        self.add_audio_clips_inner(track_key, &clip_states)?;
        for &key in &keys {
            self.clip_key_generator.reserve(key).unwrap();
        }
        Ok(keys)
    }

    pub fn add_track(
        &mut self,
        output: MixerTrackKey,
    ) -> Result<TimelineTrackKey, TimelineTrackOverflowError> {
        let key = self.track_key_generator.next()?;

        self.tracks.insert(key, TimelineTrack::new(output));

        let timeline_track = TimelineTrackProcessor::new(
            output,
            Arc::clone(&self.position),
            self.sample_rate,
            self.bpm_cents,
        );
        self.track_processors.push((key, DBox::new(timeline_track)));
        Ok(key)
    }
    pub fn add_tracks(
        &mut self,
        outputs: Vec<MixerTrackKey>,
    ) -> Result<Vec<TimelineTrackKey>, TimelineTrackOverflowError> {
        let count = outputs.len();

        if self.track_key_generator.remaining_keys()
            < count.try_into().or(Err(TimelineTrackOverflowError))?
        {
            return Err(TimelineTrackOverflowError);
        }

        let mut keys = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.track_key_generator.next().expect(
                "next_key() returned error, even though it reported remaining_keys() >= count",
            );
            keys.push(key);
        }

        for (&key, &output) in zip(&keys, outputs.iter()) {
            self.tracks.insert(key, TimelineTrack::new(output));
        }

        let track_processors = zip(&keys, outputs.iter())
            .map(|(&key, &output)| {
                (
                    key,
                    DBox::new(TimelineTrackProcessor::new(
                        output,
                        Arc::clone(&self.position),
                        self.sample_rate,
                        self.bpm_cents,
                    )),
                )
            })
            .collect();
        self.track_processors.push_multiple(track_processors);
        Ok(keys)
    }

    pub fn delete_track(&mut self, key: TimelineTrackKey) -> Result<(), InvalidTimelineTrackError> {
        let result = self.track_key_generator.free(key);
        if result.is_err() {
            return Err(InvalidTimelineTrackError { key });
        }
        self.track_processors.remove(key);
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
            self.track_key_generator
                .free(key)
                .expect("key_in_use() returned true, even though free() returned error");
            self.tracks.remove(&key);
        }
        self.track_processors.remove_multiple(keys);
        Ok(())
    }

    pub fn reconstruct_track(&mut self, state: &TimelineTrackState, output: MixerTrackKey) {
        let key = state.key;
        self.track_key_generator
            .reserve(key)
            .expect("Timeline track key already in use");

        self.tracks.insert(key, TimelineTrack::new(output));

        let timeline_track = TimelineTrackProcessor::new(
            output,
            Arc::clone(&self.position),
            self.sample_rate,
            self.bpm_cents,
        );
        self.track_processors.push((key, DBox::new(timeline_track)));
    }

    pub fn reconstruct_tracks<'a>(
        &mut self,
        states: impl Iterator<Item = (&'a TimelineTrackState, MixerTrackKey)>,
    ) {
        let track_processors = states.map(|(state, output)| {
            let key = state.key;
            self.track_key_generator
                .reserve(key)
                .expect("Timeline track key already in use");

            self.tracks.insert(key, TimelineTrack::new(output));

            (
                key,
                DBox::new(TimelineTrackProcessor::new(
                    output,
                    Arc::clone(&self.position),
                    self.sample_rate,
                    self.bpm_cents,
                )),
            )
        });
        self.track_processors
            .push_multiple(track_processors.collect());
    }

    pub fn key_in_use(&self, key: TimelineTrackKey) -> bool {
        self.track_key_generator.in_use(key)
    }

    pub fn remaining_keys(&self) -> u32 {
        self.track_key_generator.remaining_keys()
    }

    pub fn track_state(
        &self,
        key: TimelineTrackKey,
    ) -> Result<TimelineTrackState, InvalidTimelineTrackError> {
        if !self.key_in_use(key) {
            return Err(InvalidTimelineTrackError { key });
        }

        let track = self.tracks.get(&key).unwrap();
        let clips = track.clips.values().map(|clip| clip.state()).collect();
        let output_track = track.output_track;

        Ok(TimelineTrackState {
            key,
            clips,
            output_track,
        })
    }

    pub fn state(&self) -> TimelineState {
        TimelineState {
            bpm_cents: self.bpm_cents,
            audio_clip_store: self.clip_store.state(),
            tracks: self
                .tracks
                .keys()
                .map(|&key| self.track_state(key).unwrap())
                .collect(),
        }
    }
}

pub struct TimelineProcessor {
    sample_rate: u32,
    bpm_cents: u16,

    playing: Arc<AtomicBool>,
    position: Arc<AtomicU64>,

    tracks: RemotePushedHashMap<TimelineTrackKey, DBox<TimelineTrackProcessor>>,

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
                    Event::JumpTo(pos) => self.jump_to(pos),
                    Event::AddClip { track_key, clip } => self.add_clip(track_key, clip),
                    Event::AddClips { track_key, clips } => self.add_clips(track_key, clips),
                    Event::DeleteClip {
                        track_key,
                        clip_start,
                    } => self.delete_clip(track_key, clip_start),
                    Event::DeleteClips {
                        track_key,
                        clip_starts,
                    } => self.delete_clips(track_key, clip_starts),
                },
            }
        }
    }

    fn jump_to(&mut self, pos: Timestamp) {
        let pos_samples = pos.samples(self.sample_rate, self.bpm_cents);
        self.position.store(pos_samples, Ordering::Relaxed);
        for track in self.tracks.values_mut() {
            track.jump_to(pos);
        }
    }

    fn add_clip(
        &mut self,
        track_key: TimelineTrackKey,
        timeline_clip: Box<TreeNode<AudioClipProcessor>>,
    ) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        track.insert_clip(timeline_clip);
    }
    #[allow(clippy::vec_box)]
    fn add_clips(
        &mut self,
        track_key: TimelineTrackKey,
        mut timeline_clips: DBox<Vec<Box<TreeNode<AudioClipProcessor>>>>,
    ) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        for clips in timeline_clips.drain(..) {
            track.insert_clip(clips);
        }
    }

    fn delete_clip(&mut self, track_key: TimelineTrackKey, clip_start: Timestamp) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        track.delete_clip(clip_start);
    }
    fn delete_clips(&mut self, track_key: TimelineTrackKey, clip_starts: DBox<Vec<Timestamp>>) {
        let track = self
            .tracks
            .get_mut(&track_key)
            .expect("Track doesn't exist");

        track.delete_clips(clip_starts);
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TimelineState {
    pub bpm_cents: u16,
    pub audio_clip_store: AudioClipStoreState,
    pub tracks: Vec<TimelineTrackState>,
}
impl Default for TimelineState {
    /// Create an empty timeline with a BPM of 120
    fn default() -> Self {
        Self {
            bpm_cents: 120_00,
            audio_clip_store: Default::default(),
            tracks: Default::default(),
        }
    }
}
impl PartialEq for TimelineState {
    fn eq(&self, other: &Self) -> bool {
        let self_set: HashSet<_> = HashSet::from_iter(self.tracks.iter());
        let other_set: HashSet<_> = HashSet::from_iter(other.tracks.iter());

        debug_assert_eq!(
            self_set.len(),
            self.tracks.len(),
            "Duplicate timeline tracks in TimelineState: {:?}",
            self.tracks
        );
        debug_assert_eq!(
            other_set.len(),
            other.tracks.len(),
            "Duplicate timeline tracks in TimelineState: {:?}",
            other.tracks
        );

        self.bpm_cents == other.bpm_cents
            && self.audio_clip_store == other.audio_clip_store
            && self_set == other_set
    }
}
impl Eq for TimelineState {}

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
    Overlapping,
}
impl Display for AddClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTimelineTrack(key) => {
                write!(f, "No timeline track with key, {key}, on timeline")
            }
            Self::InvalidClip(key) => write!(f, "No stored audio clip with key, {key}"),
            Self::Overlapping => write!(f, "Clip overlaps with another clip"),
        }
    }
}
impl Error for AddClipError {}

#[derive(Debug, PartialEq, Eq)]
pub enum InvalidAudioClipError {
    InvalidTrack {
        track_key: TimelineTrackKey,
    },
    InvalidClip {
        track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
    },
}
impl Display for InvalidAudioClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidAudioClipError::InvalidTrack { track_key } => {
                write!(f, "Attempted to access an audio clip on a non-existing timeline track with key, {track_key}")
            }
            InvalidAudioClipError::InvalidClip {
                track_key,
                clip_key,
            } => {
                write!(
                    f,
                    "No clip with key, {clip_key}, on track with key, {track_key}"
                )
            }
        }
    }
}
impl Error for InvalidAudioClipError {}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioClipReconstructionError {
    InvalidTrack(TimelineTrackKey),
    InvalidStoredClip(StoredAudioClipKey),
    KeyInUse(AudioClipKey),
    Overlapping,
}
impl From<AddClipError> for AudioClipReconstructionError {
    fn from(err: AddClipError) -> Self {
        match err {
            AddClipError::InvalidTimelineTrack(key) => Self::InvalidTrack(key),
            AddClipError::InvalidClip(key) => Self::InvalidStoredClip(key),
            AddClipError::Overlapping => Self::Overlapping,
        }
    }
}
impl Display for AudioClipReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioClipReconstructionError::InvalidTrack(key) => {
                write!(f, "No timeline track with key, {key}, on timeline")
            }
            AudioClipReconstructionError::InvalidStoredClip(key) => {
                write!(f, "No stored audio clip with key, {key}")
            }
            AudioClipReconstructionError::KeyInUse(key) => {
                write!(f, "Clip key, {key}, already in use")
            }
            AudioClipReconstructionError::Overlapping => {
                write!(f, "Clip overlaps with another clip")
            }
        }
    }
}
impl Error for AudioClipReconstructionError {}

#[cfg(test)]
mod tests {
    use crate::engine::utils::test_file_path;

    use super::*;

    #[test]
    fn add_track() {
        let (mut tl, mut tlp, ie) = timeline(&TimelineState::default(), 40_000, 10);
        assert!(ie.is_empty());

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
        let (mut tl, mut tlp, ie) = timeline(&TimelineState::default(), 40_000, 10);
        assert!(ie.is_empty());

        let ck = tl
            .import_audio_clip(&test_file_path("44100 16-bit.wav"))
            .unwrap();
        let tk = tl.add_track(0).unwrap();
        tl.add_audio_clip(tk, ck, Timestamp::from_beat_units(0), None)
            .unwrap();

        no_heap! {{
            tlp.poll();
        }}
    }

    #[test]
    fn add_overlapping() {
        let (mut tl, _, ie) = timeline(&TimelineState::default(), 40_000, 10);
        assert!(ie.is_empty());

        let ck = tl
            .import_audio_clip(&test_file_path("44100 16-bit.wav"))
            .unwrap();
        let tk = tl.add_track(0).unwrap();

        tl.add_audio_clip(
            tk,
            ck,
            Timestamp::from_beat_units(42),
            Some(Timestamp::from_beat_units(8)),
        )
        .unwrap();
        tl.add_audio_clip(tk, ck, Timestamp::from_beat_units(50), None)
            .unwrap();
        let res = tl.add_audio_clip(
            tk,
            ck,
            Timestamp::from_beat_units(0),
            Some(Timestamp::from_beat_units(43)),
        );

        assert_eq!(res, Err(AddClipError::Overlapping));
    }
}
