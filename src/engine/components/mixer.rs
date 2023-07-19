use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;

use super::track::MixerTrackKey;
use super::track::{
    mixer_track, mixer_track_from_state, MixerTrack, MixerTrackProcessor, MixerTrackState,
};
use super::MixPoint;
use crate::engine::traits::Effect;
use crate::engine::traits::Info;
use crate::engine::utils::dropper::DBox;
use crate::engine::utils::key_generator;
use crate::engine::utils::key_generator::KeyGenerator;
use crate::engine::utils::remote_push::RemotePushable;
use crate::engine::utils::remote_push::{RemotePushedHashMap, RemotePusherHashMap};
use crate::engine::Sample;
use crate::engine::CHANNELS;

pub fn mixer(max_buffer_size: usize) -> (Mixer, MixerProcessor) {
    let key_generator = KeyGenerator::new();

    let tracks = HashMap::new();

    let (track_processors_pusher, track_processors_pushed) = HashMap::remote_push();
    let (source_outs_pusher, source_outs_pushed) = HashMap::remote_push();

    let (master, master_processor) = mixer_track(0, max_buffer_size);

    (
        Mixer {
            max_buffer_size,
            key_generator,
            tracks,
            master,

            track_processors: track_processors_pusher,
            source_outs: source_outs_pusher,
        },
        MixerProcessor {
            tracks: track_processors_pushed,
            master: DBox::new(master_processor),
            mix_point: MixPoint::new(max_buffer_size),
            source_outs: source_outs_pushed,
        },
    )
}

pub struct Mixer {
    max_buffer_size: usize,
    key_generator: KeyGenerator<MixerTrackKey>,
    tracks: HashMap<MixerTrackKey, MixerTrack>,
    master: MixerTrack,

    track_processors: RemotePusherHashMap<MixerTrackKey, DBox<MixerTrackProcessor>>,
    source_outs: RemotePusherHashMap<MixerTrackKey, DBox<Vec<Sample>>>,
}
impl Mixer {
    pub fn master(&self) -> &MixerTrack {
        &self.master
    }
    pub fn master_mut(&mut self) -> &mut MixerTrack {
        &mut self.master
    }

    pub fn track(&self, key: MixerTrackKey) -> Result<&MixerTrack, InvalidMixerTrackError> {
        self.tracks.get(&key).ok_or(InvalidMixerTrackError { key })
    }
    pub fn track_mut(
        &mut self,
        key: MixerTrackKey,
    ) -> Result<&mut MixerTrack, InvalidMixerTrackError> {
        self.tracks
            .get_mut(&key)
            .ok_or(InvalidMixerTrackError { key })
    }

    pub fn add_track(&mut self) -> Result<MixerTrackKey, MixerTrackOverflowError> {
        let key = self.key_generator.next()?;
        let track = mixer_track(key, self.max_buffer_size);
        self.push_track(track);
        Ok(key)
    }
    pub fn add_tracks(
        &mut self,
        count: u32,
    ) -> Result<Vec<MixerTrackKey>, MixerTrackOverflowError> {
        if self.key_generator.remaining_keys() < count {
            return Err(MixerTrackOverflowError);
        }

        let count = count.try_into().or(Err(MixerTrackOverflowError))?;
        let mut keys = Vec::with_capacity(count);
        let mut tracks = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.key_generator.next().expect(
                "next_key() returned error, even though it reported remaining_keys() >= count",
            );
            keys.push(key);
            let track = mixer_track(key, self.max_buffer_size);
            tracks.push(track);
        }
        self.push_tracks(tracks);
        Ok(keys)
    }

    pub fn reconstruct_track(&mut self, state: &MixerTrackState) {
        let key = state.key;
        self.key_generator
            .reserve(key)
            .expect("Track key already in use");

        let track = mixer_track_from_state(self.max_buffer_size, state);
        self.push_track(track);
    }
    pub fn reconstruct_tracks<'a>(&mut self, states: impl Iterator<Item = &'a MixerTrackState>) {
        let tracks = states
            .map(|state| {
                self.key_generator
                    .reserve(state.key)
                    .expect("Track key already in use");
                mixer_track_from_state(self.max_buffer_size, state)
            })
            .collect();
        self.push_tracks(tracks);
    }

    pub fn delete_track(&mut self, key: MixerTrackKey) -> Result<(), InvalidMixerTrackError> {
        let result = self.key_generator.free(key);
        if result.is_err() {
            return Err(InvalidMixerTrackError { key });
        }

        self.tracks.remove(&key);
        self.track_processors.remove(key);

        Ok(())
    }
    pub fn delete_tracks(
        &mut self,
        keys: Vec<MixerTrackKey>,
    ) -> Result<(), InvalidMixerTrackError> {
        for &key in &keys {
            if !self.tracks.contains_key(&key) {
                return Err(InvalidMixerTrackError { key });
            }
        }
        for key in &keys {
            self.key_generator
                .free(*key)
                .expect("At least one key exists in tracks but not in key_generator");
            self.tracks.remove(key);
        }
        self.track_processors.remove_multiple(keys);
        Ok(())
    }

    fn push_track(&mut self, track: (MixerTrack, MixerTrackProcessor)) {
        let (track, track_processor) = track;
        let key = track.key();
        self.tracks.insert(key, track);
        self.source_outs
            .push((key, DBox::new(vec![0.0; self.max_buffer_size * CHANNELS])));
        self.track_processors
            .push((key, DBox::new(track_processor)))
    }
    fn push_tracks(&mut self, tracks: Vec<(MixerTrack, MixerTrackProcessor)>) {
        let mut track_processors = vec![];
        let mut source_outs = vec![];
        for track in tracks {
            let (track, track_processor) = track;
            let key = track.key();
            self.tracks.insert(key, track);

            track_processors.push((key, DBox::new(track_processor)));
            source_outs.push((key, DBox::new(vec![0.0; self.max_buffer_size * CHANNELS])));
        }
        self.source_outs.push_multiple(source_outs);
        self.track_processors.push_multiple(track_processors);
    }

    pub fn key_in_use(&self, key: MixerTrackKey) -> bool {
        self.key_generator.in_use(key)
    }

    pub fn remaining_keys(&self) -> u32 {
        self.key_generator.remaining_keys()
    }
}

pub struct MixerProcessor {
    tracks: RemotePushedHashMap<MixerTrackKey, DBox<MixerTrackProcessor>>,
    master: DBox<MixerTrackProcessor>,
    source_outs: RemotePushedHashMap<MixerTrackKey, DBox<Vec<Sample>>>,
    mix_point: MixPoint,
}
impl MixerProcessor {
    pub fn source_outs(&mut self) -> &mut HashMap<MixerTrackKey, DBox<Vec<Sample>>> {
        &mut self.source_outs
    }

    pub fn poll(&mut self) {
        self.tracks.poll();
        self.source_outs.poll();
    }

    pub fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate: _,
            buffer_size,
        } = *info;

        self.mix_point.reset();
        for (key, track) in self.tracks.iter_mut() {
            let buffer = self.source_outs.get_mut(key).expect("Track has no input");
            track.process(info, buffer);
            self.mix_point.add(buffer);
        }
        let out = &mut self.mix_point.get()[..buffer_size * CHANNELS];

        self.master.process(info, out);
        out
    }
}
impl Debug for MixerProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MixerProcessor {{ tracks: {:?}, ... }}", self.tracks)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidMixerTrackError {
    pub key: MixerTrackKey,
}
impl Display for InvalidMixerTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No track with key, {}, on mixer", self.key)
    }
}
impl Error for InvalidMixerTrackError {}

#[derive(Debug, PartialEq, Eq)]
pub struct MixerTrackOverflowError;
impl Display for MixerTrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of tracks has been exceeded. Impressive")
    }
}
impl Error for MixerTrackOverflowError {}
impl From<key_generator::OverflowError> for MixerTrackOverflowError {
    fn from(_: key_generator::OverflowError) -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_track() {
        let (mut m, mut mp) = mixer(10);

        for _ in 0..50 {
            m.add_track().unwrap();
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(m.tracks.len(), 50);
        assert_eq!(mp.tracks.len(), 50);
    }

    #[test]
    fn add_tracks() {
        let (mut m, mut mp) = mixer(10);

        for _ in 0..50 {
            m.add_tracks(5).unwrap();
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(m.tracks.len(), 50 * 5);
        assert_eq!(mp.tracks.len(), 50 * 5);
    }

    #[test]
    fn reconstruct_track() {
        let (mut m, mut mp) = mixer(10);

        let mut keys = Vec::new();
        for _ in 0..50 {
            keys.push(m.add_track().unwrap());
        }

        let mut states = Vec::new();
        for key in keys {
            let state = m.track(key).unwrap().state();
            m.delete_track(key).unwrap();
            states.push(state);
        }

        for state in states {
            m.reconstruct_track(&state);
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(mp.tracks.len(), 50);
    }
    #[test]
    #[should_panic]
    fn reconstruct_existing_track() {
        let (mut m, _mp) = mixer(10);

        let used = m.add_track().unwrap();

        m.reconstruct_track(&MixerTrackState {
            panning: 0.0,
            volume: 1.0,

            key: used,
        });
    }

    #[test]
    fn reconstruct_tracks() {
        let (mut m, mut mp) = mixer(10);

        let batch_size = 5;

        let states: Vec<MixerTrackState> = (1..50 * batch_size + 1)
            .map(|key| MixerTrackState {
                panning: 0.0,
                volume: 1.0,
                key: key as MixerTrackKey,
            })
            .collect();

        for state in states.chunks(batch_size) {
            m.reconstruct_tracks(state.iter());
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(mp.tracks.len(), 50 * batch_size);
    }

    #[test]
    fn delete_track_immediately() {
        let (mut m, mut mp) = mixer(10);

        let k = m.add_track().unwrap();
        m.delete_track(k).unwrap();

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(m.tracks.len(), 0);
        assert_eq!(mp.tracks.len(), 0);
    }

    #[test]
    fn delete_track_delayed() {
        let (mut m, mut mp) = mixer(10);

        let mut poll = || {
            no_heap! {{
                mp.poll();
            }}
        };

        let k = m.add_track().unwrap();
        poll();
        m.delete_track(k).unwrap();
        poll();

        assert_eq!(m.tracks.len(), 0);
        assert_eq!(mp.tracks.len(), 0);
    }
}
