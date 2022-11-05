use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;

use super::event_queue::EventQueue;
use super::event_queue::EventReceiver;
use super::track::{track, track_from_data, Track, TrackData, TrackProcessor};
use super::MixPoint;
use crate::engine::traits::{Component, Info, Source};
use crate::engine::utils::remote_push::RemotePushable;
use crate::engine::utils::remote_push::{RemotePushedHashMap, RemotePusherHashMap};
use crate::engine::Sample;

pub type TrackKey = u32;

pub fn mixer(event_queue: &mut EventQueue, max_buffer_size: usize) -> (Mixer, MixerProcessor) {
    let (track, track_processor) = track(0, max_buffer_size);

    let mut tracks = HashMap::new();
    tracks.insert(0, track);

    let (track_processors_pusher, mut track_processors_pushed) = HashMap::remote_push(event_queue);
    track_processors_pushed.insert(0, track_processor);

    (
        Mixer {
            max_buffer_size,

            tracks,
            last_key: 0,

            track_processors: track_processors_pusher,
        },
        MixerProcessor {
            tracks: track_processors_pushed,
            mix_point: MixPoint::new(max_buffer_size),
        },
    )
}

pub struct Mixer {
    max_buffer_size: usize,

    tracks: HashMap<TrackKey, Track>,
    last_key: TrackKey,

    track_processors: RemotePusherHashMap<TrackKey, TrackProcessor>,
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
        let key = self.next_key()?;
        let track = track(key, self.max_buffer_size);
        self.push_track(track);
        Ok(key)
    }
    pub fn add_tracks(&mut self, count: TrackKey) -> Result<Vec<TrackKey>, TrackOverflowError> {
        let count = count.try_into().or(Err(TrackOverflowError {}))?;
        let keys = self.next_keys(count)?;
        let mut tracks = Vec::with_capacity(count);
        for &key in &keys {
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

        if self.tracks.contains_key(&key) {
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
        let result = self.tracks.remove(&key);
        if result.is_none() {
            return Err(InvalidTrackError { key });
        }

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
            self.tracks.remove(key);
        }
        self.track_processors.remove_multiple(keys);
        Ok(())
    }

    fn next_key_after(&self, last_key: TrackKey) -> Result<TrackKey, TrackOverflowError> {
        let mut key = last_key.wrapping_add(1);

        let mut i = 0;
        while self.tracks.contains_key(&key) {
            i += 1;
            if i == TrackKey::MAX {
                return Err(TrackOverflowError);
            }

            key = key.wrapping_add(1);
        }

        Ok(key)
    }
    fn next_key(&mut self) -> Result<TrackKey, TrackOverflowError> {
        let key = self.next_key_after(self.last_key)?;
        self.last_key = key;
        Ok(key)
    }
    fn next_keys(&mut self, count: usize) -> Result<Vec<TrackKey>, TrackOverflowError> {
        let mut keys = Vec::with_capacity(count);
        let mut last_key = self.last_key;
        for _ in 0..count {
            let key = self.next_key_after(last_key)?;
            keys.push(key);
            last_key = key;
        }

        // Only commit once we know enough are available
        self.last_key = last_key;
        Ok(keys)
    }

    fn push_track(&mut self, track: (Track, TrackProcessor)) {
        let (track, track_processor) = track;
        let key = track.key();
        self.tracks.insert(key, track);
        self.track_processors.push((key, track_processor))
    }
    fn push_tracks(&mut self, tracks: Vec<(Track, TrackProcessor)>) {
        let count = tracks.len();

        let mut track_processors = Vec::with_capacity(count);
        for track in tracks {
            let (track, track_processor) = track;
            let key = track.key();
            self.tracks.insert(key, track);
            track_processors.push((key, track_processor));
        }
        self.track_processors.push_multiple(track_processors);
    }
}

pub struct MixerProcessor {
    tracks: RemotePushedHashMap<TrackKey, TrackProcessor>,
    mix_point: MixPoint,
}
impl Debug for MixerProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MixerProcessor {{tracks: {:?}, ...}}", self.tracks)
    }
}
impl Component for MixerProcessor {
    fn poll<'a, 'b>(&'a mut self, event_receiver: &mut EventReceiver<'a, 'b>) {
        self.tracks.poll(event_receiver);
    }
}
impl Source for MixerProcessor {
    fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate: _,
            buffer_size,
        } = *info;

        self.mix_point.reset();
        for track in self.tracks.values_mut() {
            let buffer = track.output(info);
            self.mix_point.add(buffer);
        }
        match self.mix_point.get() {
            Ok(buffer) => buffer,
            Err(buffer) => &mut buffer[..buffer_size],
        }
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
    use crate::engine::components::event_queue::event_queue;

    use super::*;

    #[test]
    fn add_track() {
        let (mut eq, mut eqp) = event_queue();
        let (mut m, mut mp) = mixer(&mut eq, 10);

        for _ in 0..50 {
            m.add_track().unwrap();
        }
        let mut ec = eqp.event_consumer();
        mp.poll(&mut ec);
        ec.poll();

        assert_eq!(m.tracks.len(), 51);
        assert_eq!(mp.tracks.len(), 51);
    }

    #[test]
    fn add_tracks() {
        let (mut eq, mut eqp) = event_queue();
        let (mut m, mut mp) = mixer(&mut eq, 10);

        for _ in 0..50 {
            m.add_tracks(5).unwrap();
        }
        let mut ec = eqp.event_consumer();
        mp.poll(&mut ec);
        ec.poll();

        assert_eq!(m.tracks.len(), 50 * 5 + 1);
        assert_eq!(mp.tracks.len(), 50 * 5 + 1);
    }

    #[test]
    fn reconstruct_track() {
        let (mut eq, mut eqp) = event_queue();
        let (mut m, mut mp) = mixer(&mut eq, 10);

        for key in 1..50 + 1 {
            m.reconstruct_track(&TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .unwrap();
        }
        let mut ec = eqp.event_consumer();
        mp.poll(&mut ec);
        ec.poll();

        assert_eq!(mp.tracks.len(), 50 + 1);
    }
    #[test]
    fn reconstruct_existing_track() {
        let (mut eq, mut eqp) = event_queue();
        let (mut m, mut mp) = mixer(&mut eq, 10);

        let result = m.reconstruct_track(&TrackData {
            panning: 0.0,
            volume: 1.0,

            // Should already be in use by the initial track
            key: 0,
        });

        let mut ec = eqp.event_consumer();
        mp.poll(&mut ec);
        ec.poll();

        assert_eq!(result, Err(TrackReconstructionError { key: 0 }));
        assert_eq!(m.tracks.len(), 1);
        assert_eq!(mp.tracks.len(), 1);
    }

    #[test]
    fn reconstruct_tracks() {
        let (mut eq, mut eqp) = event_queue();
        let (mut m, mut mp) = mixer(&mut eq, 10);

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
        let mut ec = eqp.event_consumer();
        mp.poll(&mut ec);
        ec.poll();

        assert_eq!(mp.tracks.len(), 50 * batch_size + 1);
    }
}
