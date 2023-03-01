use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;

use super::track::TrackKey;
use super::track::{track, track_from_data, Track, TrackData, TrackProcessor};
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
    let mut key_generator = KeyGenerator::new();

    let (track, track_processor) = track(0, max_buffer_size);

    let mut tracks = HashMap::new();
    tracks.insert(0, track);
    key_generator.reserve(0).unwrap();

    let (mut track_processors_pusher, mut track_processors_pushed) = HashMap::remote_push();
    let (mut source_outs_pusher, mut source_outs_pushed) = HashMap::remote_push();

    track_processors_pusher.push((0, DBox::new(track_processor)));
    source_outs_pusher.push((0, vec![0.0; max_buffer_size * CHANNELS]));

    track_processors_pushed.poll();
    source_outs_pushed.poll();

    (
        Mixer {
            max_buffer_size,
            key_generator,
            tracks,
            track_processors: track_processors_pusher,
            source_outs: source_outs_pusher,
        },
        MixerProcessor {
            tracks: track_processors_pushed,
            mix_point: MixPoint::new(max_buffer_size),
            source_outs: source_outs_pushed,
        },
    )
}

pub struct Mixer {
    max_buffer_size: usize,
    key_generator: KeyGenerator<TrackKey>,
    tracks: HashMap<TrackKey, Track>,
    track_processors: RemotePusherHashMap<TrackKey, DBox<TrackProcessor>>,
    source_outs: RemotePusherHashMap<TrackKey, Vec<Sample>>,
}
impl Mixer {
    pub fn tracks(&self) -> Vec<&Track> {
        self.tracks.values().collect()
    }
    pub fn tracks_mut(&mut self) -> Vec<&mut Track> {
        self.tracks.values_mut().collect()
    }

    pub fn track(&self, key: TrackKey) -> Result<&Track, InvalidTrackError> {
        self.tracks.get(&key).ok_or(InvalidTrackError { key })
    }
    pub fn track_mut(&mut self, key: TrackKey) -> Result<&mut Track, InvalidTrackError> {
        self.tracks.get_mut(&key).ok_or(InvalidTrackError { key })
    }

    pub fn add_track(&mut self) -> Result<TrackKey, TrackOverflowError> {
        let key = self.key_generator.next()?;
        let track = track(key, self.max_buffer_size);
        self.push_track(track);
        Ok(key)
    }
    pub fn add_tracks(&mut self, count: TrackKey) -> Result<Vec<TrackKey>, TrackOverflowError> {
        if self.key_generator.remaining_keys() < count {
            return Err(TrackOverflowError);
        }

        let count = count.try_into().or(Err(TrackOverflowError))?;
        let mut keys = Vec::with_capacity(count);
        let mut tracks = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.key_generator.next().expect(
                "next_key() returned error, even though it reported remaining_keys() >= count",
            );
            keys.push(key);
            let track = track(key, self.max_buffer_size);
            tracks.push(track);
        }
        self.push_tracks(tracks);
        Ok(keys)
    }

    pub fn reconstruct_track(
        &mut self,
        data: &TrackData,
    ) -> Result<TrackKey, TrackReconstructionError> {
        let key = data.key;

        let result = self.key_generator.reserve(key);
        if result.is_err() {
            return Err(TrackReconstructionError { key });
        }

        let track = track_from_data(self.max_buffer_size, data);
        self.push_track(track);

        Ok(key)
    }
    pub fn reconstruct_tracks<'a>(
        &mut self,
        data: impl Iterator<Item = &'a TrackData>,
    ) -> Result<Vec<TrackKey>, TrackReconstructionError> {
        let (min_size, max_size) = data.size_hint();
        let size = max_size.unwrap_or(min_size);
        let mut keys = Vec::with_capacity(size);
        let mut tracks = Vec::with_capacity(size);

        for data in data {
            let key = data.key;

            if self.tracks.contains_key(&key) {
                return Err(TrackReconstructionError { key });
            }

            keys.push(key);
            tracks.push(track_from_data(self.max_buffer_size, data))
        }
        keys.shrink_to_fit();
        tracks.shrink_to_fit();

        self.push_tracks(tracks);
        Ok(keys)
    }

    pub fn delete_track(&mut self, key: TrackKey) -> Result<(), InvalidTrackError> {
        let result = self.key_generator.free(key);
        if result.is_err() {
            return Err(InvalidTrackError { key });
        }

        self.tracks.remove(&key);
        self.track_processors.remove(key);

        Ok(())
    }
    pub fn delete_tracks(&mut self, keys: Vec<TrackKey>) -> Result<(), InvalidTrackError> {
        for &key in &keys {
            if !self.tracks.contains_key(&key) {
                return Err(InvalidTrackError { key });
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

    fn push_track(&mut self, track: (Track, TrackProcessor)) {
        let (track, track_processor) = track;
        let key = track.key();
        self.tracks.insert(key, track);
        self.source_outs
            .push((key, vec![0.0; self.max_buffer_size * CHANNELS]));
        self.track_processors
            .push((key, DBox::new(track_processor)))
    }
    fn push_tracks(&mut self, tracks: Vec<(Track, TrackProcessor)>) {
        let mut track_processors = vec![];
        let mut source_outs = vec![];
        for track in tracks {
            let (track, track_processor) = track;
            let key = track.key();
            self.tracks.insert(key, track);

            track_processors.push((key, DBox::new(track_processor)));
            source_outs.push((key, vec![0.0; self.max_buffer_size * CHANNELS]));
        }
        self.source_outs.push_multiple(source_outs);
        self.track_processors.push_multiple(track_processors);
    }

    pub fn remaining_keys(&self) -> u32 {
        self.key_generator.remaining_keys()
    }
}

pub struct MixerProcessor {
    tracks: RemotePushedHashMap<TrackKey, DBox<TrackProcessor>>,
    source_outs: RemotePushedHashMap<TrackKey, Vec<Sample>>,
    mix_point: MixPoint,
}
impl MixerProcessor {
    pub fn source_outs(&mut self) -> &mut HashMap<TrackKey, Vec<Sample>> {
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
        match self.mix_point.get() {
            Ok(buffer) => buffer,
            Err(buffer) => &mut buffer[..buffer_size * CHANNELS],
        }
    }
}
impl Debug for MixerProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MixerProcessor {{ tracks: {:?}, ... }}", self.tracks)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidTrackError {
    key: TrackKey,
}
impl Display for InvalidTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No track with key: {}", self.key)
    }
}
impl Error for InvalidTrackError {}

#[derive(Debug, PartialEq, Eq)]
pub struct TrackOverflowError;
impl Display for TrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of tracks has been exceeded. Impressive")
    }
}
impl Error for TrackOverflowError {}
impl From<key_generator::OverflowError> for TrackOverflowError {
    fn from(_: key_generator::OverflowError) -> Self {
        Self
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrackReconstructionError {
    key: TrackKey,
}
impl Display for TrackReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Track with key, {}, already exists", self.key)
    }
}
impl Error for TrackReconstructionError {}

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

        assert_eq!(m.tracks.len(), 51);
        assert_eq!(mp.tracks.len(), 51);
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

        assert_eq!(m.tracks.len(), 50 * 5 + 1);
        assert_eq!(mp.tracks.len(), 50 * 5 + 1);
    }

    #[test]
    fn reconstruct_track() {
        let (mut m, mut mp) = mixer(10);

        for key in 1..50 + 1 {
            m.reconstruct_track(&TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .unwrap();
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(mp.tracks.len(), 50 + 1);
    }
    #[test]
    fn reconstruct_existing_track() {
        let (mut m, mut mp) = mixer(10);

        let result = m.reconstruct_track(&TrackData {
            panning: 0.0,
            volume: 1.0,

            // Should already be in use by the initial track
            key: 0,
        });

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(result, Err(TrackReconstructionError { key: 0 }));
        assert_eq!(m.tracks.len(), 1);
        assert_eq!(mp.tracks.len(), 1);
    }

    #[test]
    fn reconstruct_tracks() {
        let (mut m, mut mp) = mixer(10);

        let batch_size = 5;

        let data: Vec<TrackData> = (1..50 * batch_size + 1)
            .map(|key| TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .collect();

        for data in data.chunks(batch_size) {
            m.reconstruct_tracks(data.iter()).unwrap();
        }

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(mp.tracks.len(), 50 * batch_size + 1);
    }

    #[test]
    fn delete_track_immediately() {
        let (mut m, mut mp) = mixer(10);

        let k = m.add_track().unwrap();
        m.delete_track(k).unwrap();

        no_heap! {{
            mp.poll();
        }}

        assert_eq!(m.tracks.len(), 1);
        assert_eq!(mp.tracks.len(), 1);
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

        assert_eq!(m.tracks.len(), 1);
        assert_eq!(mp.tracks.len(), 1);
    }
}
