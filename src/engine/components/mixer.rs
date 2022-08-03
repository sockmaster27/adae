use core::sync::atomic::Ordering;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::mem;
use std::sync::Arc;

use atomicbox::AtomicOptionBox;

use super::track::{new_track, track_from_data, Track, TrackData, TrackProcessor};
use super::MixPoint;
use crate::engine::Sample;

pub type TrackKey = u32;

/// Size of all ringbuffers. Determines how many times `Mixer::add_track(s)`
/// and `Mixer::delete_track(s)` can each be called inbetween polls (buffer outputs).
const CAPACITY: usize = 64;

pub fn new_mixer(max_buffer_size: usize) -> (Mixer, MixerProcessor) {
    let new_map1 = Arc::new(AtomicOptionBox::none());
    let new_map2 = Arc::clone(&new_map1);

    let (added_tracks_p, added_tracks_c) = ringbuf::RingBuffer::new(CAPACITY).split();
    let (added_tracks_batches_p, added_tracks_batches_c) =
        ringbuf::RingBuffer::new(CAPACITY).split();

    let (deleted_tracks_p, deleted_tracks_c) = ringbuf::RingBuffer::new(CAPACITY).split();
    let (deleted_track_batches_p, deleted_track_batches_c) =
        ringbuf::RingBuffer::new(CAPACITY).split();

    let (track, track_processor) = new_track(0, max_buffer_size);

    let mut tracks = HashMap::new();
    tracks.insert(0, track);

    let mut tracks_processor = HashMap::new();
    tracks_processor.insert(0, track_processor);
    let proc_tracks_cap = tracks_processor.capacity();

    (
        Mixer {
            max_buffer_size,

            tracks,
            last_key: 0,

            added_tracks: added_tracks_p,
            added_track_batches: added_tracks_batches_p,

            deleted_tracks: deleted_tracks_p,
            deleted_track_batches: deleted_track_batches_p,

            proc_tracks_cap,
            new_proc_tracks: new_map1,
        },
        MixerProcessor {
            tracks: tracks_processor,
            mix_point: MixPoint::new(max_buffer_size),

            added_tracks: added_tracks_c,
            added_track_batches: added_tracks_batches_c,

            deleted_tracks: deleted_tracks_c,
            deleted_track_batches: deleted_track_batches_c,

            new_map: new_map2,
        },
    )
}

pub struct Mixer {
    max_buffer_size: usize,

    tracks: HashMap<TrackKey, Track>,
    last_key: TrackKey,

    added_tracks: ringbuf::Producer<(TrackKey, TrackProcessor)>,
    added_track_batches: ringbuf::Producer<Vec<(TrackKey, TrackProcessor)>>,

    deleted_tracks: ringbuf::Producer<TrackKey>,
    deleted_track_batches: ringbuf::Producer<Vec<TrackKey>>,

    proc_tracks_cap: usize,
    new_proc_tracks: Arc<AtomicOptionBox<HashMap<TrackKey, TrackProcessor>>>,
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
        let track = new_track(key, self.max_buffer_size);
        self.push_track(track);
        Ok(key)
    }
    pub fn add_tracks(&mut self, count: TrackKey) -> Result<Vec<TrackKey>, TrackOverflowError> {
        let count = count.try_into().or(Err(TrackOverflowError {}))?;
        let keys = self.next_keys(count)?;
        let mut tracks = Vec::with_capacity(count);
        for &key in &keys {
            let track = new_track(key, self.max_buffer_size);
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

        self.deleted_tracks
            .push(key)
            .expect("`Mixer::delete_track` was called too many times inbetween polls. See `Mixer::delete_tracks`");

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
        self.deleted_track_batches
            .push(keys)
            .expect("`Mixer::delete_tracks` was called too many times inbetween polls");
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

        self.ensure_capacity(1);

        self.tracks.insert(key, track);
        self.added_tracks.push((key, track_processor))
            .expect("`Mixer::push_track` was called too many times inbetween polls. See `Mixer::push_tracks`");
    }
    fn push_tracks(&mut self, tracks: Vec<(Track, TrackProcessor)>) {
        let count = tracks.len();
        self.ensure_capacity(count);

        let mut track_processors = Vec::with_capacity(count);
        for track in tracks {
            let (track, track_processor) = track;
            let key = track.key();
            self.tracks.insert(key, track);
            track_processors.push((key, track_processor));
        }
        self.added_track_batches
            .push(track_processors)
            .expect("`Mixer::push_tracks` was called too many times inbetween polls");
    }

    /// Do the work of reallocating `MixerProcessor`'s `tracks` if needed
    fn ensure_capacity(&mut self, new_track_count: usize) {
        if self.tracks.len() + new_track_count > self.proc_tracks_cap {
            self.proc_tracks_cap *= 2;
            self.new_proc_tracks.store(
                Some(Box::new(HashMap::with_capacity(self.proc_tracks_cap))),
                Ordering::SeqCst,
            );
        }
    }
}

pub struct MixerProcessor {
    tracks: HashMap<TrackKey, TrackProcessor>,
    mix_point: MixPoint,

    added_tracks: ringbuf::Consumer<(TrackKey, TrackProcessor)>,
    added_track_batches: ringbuf::Consumer<Vec<(TrackKey, TrackProcessor)>>,

    deleted_tracks: ringbuf::Consumer<TrackKey>,
    deleted_track_batches: ringbuf::Consumer<Vec<TrackKey>>,

    new_map: Arc<AtomicOptionBox<HashMap<TrackKey, TrackProcessor>>>,
}
impl MixerProcessor {
    pub fn poll(&mut self) {
        let new_map = self.new_map.take(Ordering::SeqCst);
        if let Some(new_map) = new_map {
            let old_map = mem::replace(&mut self.tracks, *new_map);
            for (key, track) in old_map {
                self.tracks.insert(key, track);
            }
        }

        self.added_tracks.pop_each(
            |(key, track)| {
                self.tracks.insert(key, track);
                true
            },
            None,
        );
        self.added_track_batches.pop_each(
            |batch| {
                for (key, track) in batch {
                    self.tracks.insert(key, track);
                }
                true
            },
            None,
        );

        self.deleted_tracks.pop_each(
            |key| {
                self.tracks.remove(&key);
                true
            },
            None,
        );
        self.deleted_track_batches.pop_each(
            |batch| {
                for key in batch {
                    self.tracks.remove(&key);
                }
                true
            },
            None,
        );

        for track in self.tracks.values_mut() {
            track.poll();
        }

        // TODO: Drop old_map and batches in another thread?
    }

    pub fn output(&mut self, sample_rate: TrackKey, buffer_size: usize) -> &mut [Sample] {
        self.mix_point.reset();
        for track in self.tracks.values_mut() {
            let buffer = track.output(sample_rate, buffer_size);
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
    use super::*;

    #[test]
    fn add_track() {
        let (mut m, mut mp) = new_mixer(10);

        for _ in 0..CAPACITY {
            m.add_track().unwrap();
        }
        mp.poll();

        assert_eq!(m.tracks.len(), CAPACITY + 1);
        assert_eq!(mp.tracks.len(), CAPACITY + 1);
    }
    #[test]
    #[should_panic]
    fn add_track_too_many_times() {
        let (mut m, _mp) = new_mixer(10);

        for _ in 0..CAPACITY + 1 {
            m.add_track().unwrap();
        }
    }

    #[test]
    fn add_tracks() {
        let (mut m, mut mp) = new_mixer(10);

        for _ in 0..CAPACITY {
            m.add_tracks(5).unwrap();
        }
        mp.poll();

        assert_eq!(m.tracks.len(), CAPACITY * 5 + 1);
        assert_eq!(mp.tracks.len(), CAPACITY * 5 + 1);
    }
    #[test]
    #[should_panic]
    fn add_tracks_too_many_times() {
        let (mut m, _mp) = new_mixer(10);

        for _ in 0..CAPACITY + 1 {
            m.add_tracks(5).unwrap();
        }
    }

    #[test]
    fn reconstruct_track() {
        let (mut m, mut mp) = new_mixer(10);

        for key in 1..CAPACITY + 1 {
            m.reconstruct_track(&TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .unwrap();
        }
        mp.poll();

        assert_eq!(mp.tracks.len(), CAPACITY + 1);
    }
    #[test]
    #[should_panic]
    fn reconstruct_track_too_many_times() {
        let (mut m, _mp) = new_mixer(10);

        for key in 1..CAPACITY + 2 {
            m.reconstruct_track(&TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .unwrap();
        }
    }
    #[test]
    fn reconstruct_existing_track() {
        let (mut m, mut mp) = new_mixer(10);

        let result = m.reconstruct_track(&TrackData {
            panning: 0.0,
            volume: 1.0,

            // Should already be in use by the initial track
            key: 0,
        });

        mp.poll();

        assert_eq!(result, Err(TrackReconstructionError { key: 0 }));
        assert_eq!(m.tracks.len(), 1);
        assert_eq!(mp.tracks.len(), 1);
    }

    #[test]
    fn reconstruct_tracks() {
        let (mut m, mut mp) = new_mixer(10);

        let batch_size = 5;

        let data: Vec<TrackData> = (1..CAPACITY * batch_size + 1)
            .map(|key| TrackData {
                panning: 0.0,
                volume: 1.0,
                key: key as TrackKey,
            })
            .collect();

        for data in data.chunks(batch_size) {
            m.reconstruct_tracks(data.iter()).unwrap();
        }
        mp.poll();

        assert_eq!(mp.tracks.len(), CAPACITY * batch_size + 1);
    }
}
