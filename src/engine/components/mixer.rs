use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use atomicbox::AtomicOptionBox;

use super::mixer_track::{mixer_track_from_data, new_mixer_track, MixerTrack, MixerTrackInterface};
use super::MixPoint;
use crate::engine::Sample;
use crate::MixerTrackData;

pub fn new_mixer(max_buffer_size: usize) -> (MixerInterface, Mixer) {
    let new_map1 = Arc::new(AtomicOptionBox::none());
    let new_map2 = Arc::clone(&new_map1);

    let (new_tracks_p, new_tracks_c) = ringbuf::RingBuffer::new(10).split();
    let (deleted_tracks_p, _deleted_tracks_c) = ringbuf::RingBuffer::new(10).split();

    let (track_interface, track) = new_mixer_track(0, max_buffer_size);

    let mut track_interfaces = HashMap::new();
    track_interfaces.insert(0, track_interface);

    let mut tracks = HashMap::new();
    tracks.insert(0, track);
    let cap = tracks.capacity();

    (
        MixerInterface {
            max_buffer_size,

            tracks: track_interfaces,
            last_key: 0,

            cap,
            new_map: new_map1,
            new_tracks: new_tracks_p,

            deleted_tracks: deleted_tracks_p,
        },
        Mixer {
            tracks,
            mix_point: MixPoint::new(max_buffer_size),

            new_map: new_map2,
            new_tracks: new_tracks_c,

            deleted_tracks: _deleted_tracks_c,
        },
    )
}

pub struct Mixer {
    tracks: HashMap<u32, MixerTrack>,
    mix_point: MixPoint,

    new_map: Arc<AtomicOptionBox<HashMap<u32, MixerTrack>>>,
    new_tracks: ringbuf::Consumer<(u32, MixerTrack)>,

    deleted_tracks: ringbuf::Consumer<u32>,
}
impl Mixer {
    pub fn poll(&mut self) {
        let new_map = self.new_map.take(Ordering::SeqCst);
        if let Some(new_map) = new_map {
            let old_map = mem::replace(&mut self.tracks, *new_map);
            for (key, track) in old_map {
                self.tracks.insert(key, track);
            }
        }
        // TODO: Drop old_map in another thread?

        self.new_tracks.pop_each(
            |(key, track)| {
                self.tracks.insert(key, track);
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
    }

    pub fn output(&mut self, sample_rate: u32, buffer_size: usize) -> &mut [Sample] {
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

pub struct MixerInterface {
    max_buffer_size: usize,

    tracks: HashMap<u32, MixerTrackInterface>,
    last_key: u32,

    cap: usize,
    new_map: Arc<AtomicOptionBox<HashMap<u32, MixerTrack>>>,
    new_tracks: ringbuf::Producer<(u32, MixerTrack)>,

    deleted_tracks: ringbuf::Producer<u32>,
}
impl MixerInterface {
    pub fn tracks(&self) -> Vec<&MixerTrackInterface> {
        self.tracks.values().collect()
    }
    pub fn tracks_mut(&mut self) -> Vec<&mut MixerTrackInterface> {
        self.tracks.values_mut().collect()
    }

    pub fn track(&self, key: u32) -> Result<&MixerTrackInterface, InvalidTrackError> {
        self.tracks.get(&key).ok_or(InvalidTrackError { key })
    }
    pub fn track_mut(&mut self, key: u32) -> Result<&mut MixerTrackInterface, InvalidTrackError> {
        self.tracks.get_mut(&key).ok_or(InvalidTrackError { key })
    }

    pub fn add_track(&mut self) -> Result<&mut MixerTrackInterface, TrackOverflowError> {
        // Find next available key
        let mut key = self.last_key.wrapping_add(1);

        let mut i = 0;
        while self.tracks.contains_key(&key) {
            i += 1;
            if i == u32::MAX {
                return Err(TrackOverflowError());
            }

            key = key.wrapping_add(1);
        }
        self.last_key = key;

        let (track_interface, track) = new_mixer_track(key, self.max_buffer_size);
        self.tracks.insert(key, track_interface);

        // Do the work of reallocating Mixer's HashMap if needed
        if self.tracks.len() > self.cap {
            self.cap *= 2;
            self.new_map.store(
                Some(Box::new(HashMap::with_capacity(self.cap))),
                Ordering::SeqCst,
            );
        }

        self.new_tracks
            .push((key, track))
            .expect("Too many tracks added to mixer inbetween polls");

        let track_interface = self.tracks.get_mut(&key).unwrap();
        Ok(track_interface)
    }

    pub fn reconstruct_track(
        &mut self,
        data: &MixerTrackData,
    ) -> Result<&mut MixerTrackInterface, TrackReconstructionError> {
        let key = data.key;

        if self.tracks.contains_key(&key) {
            return Err(TrackReconstructionError { key });
        }

        let (track_interface, track) = mixer_track_from_data(self.max_buffer_size, data);
        self.tracks.insert(key, track_interface);

        self.new_tracks
            .push((key, track))
            .expect("Too many tracks added to mixer inbetween polls");

        let track_interface = self.tracks.get_mut(&key).unwrap();
        Ok(track_interface)
    }

    pub fn delete_track(&mut self, key: u32) -> Result<(), InvalidTrackError> {
        let result = self.tracks.remove(&key);
        if result.is_none() {
            return Err(InvalidTrackError { key });
        }

        self.deleted_tracks
            .push(key)
            .expect("Too many tracks deleted from mixer inbetween polls");

        Ok(())
    }
}

#[derive(Debug)]
pub struct InvalidTrackError {
    key: u32,
}
impl Display for InvalidTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No track with key: {}", self.key)
    }
}
impl Error for InvalidTrackError {}

#[derive(Debug)]
pub struct TrackOverflowError();
impl Display for TrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of tracks has been exceeded. Impressive")
    }
}
impl Error for TrackOverflowError {}

#[derive(Debug)]
pub struct TrackReconstructionError {
    key: u32,
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
    fn add_single_track() {
        let max_buffer_size = 10;
        let (mut mi, mut m) = new_mixer(max_buffer_size);

        mi.add_track().unwrap();
        m.poll();

        assert_eq!(m.tracks.len(), 2);
    }

    #[test]
    fn add_multiple_tracks() {
        let max_buffer_size = 10;
        let (mut mi, mut m) = new_mixer(max_buffer_size);

        for _ in 0..10 {
            mi.add_track().unwrap();
        }
        m.poll();

        assert_eq!(m.tracks.len(), 11);
    }

    #[test]
    #[should_panic]
    fn add_too_many_tracks() {
        let max_buffer_size = 10;
        let (mut mi, _m) = new_mixer(max_buffer_size);

        for _ in 0..11 {
            mi.add_track().unwrap();
        }
    }
}
