use std::mem;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use atomicbox::AtomicOptionBox;

use super::mixer_track::{new_mixer_track, MixerTrack, MixerTrackInterface};
use super::MixPoint;
use crate::engine::Sample;

pub fn new_mixer(max_buffer_size: usize) -> (MixerInterface, Mixer) {
    let (track_interface, track) = new_mixer_track(max_buffer_size);
    let tracks = vec![track];

    let cap = tracks.capacity();

    let new_vec1 = Arc::new(AtomicOptionBox::none());
    let new_vec2 = Arc::clone(&new_vec1);

    let (producer, consumer) = ringbuf::RingBuffer::new(10).split();

    (
        MixerInterface {
            tracks: vec![track_interface],

            cap,
            new_vec: new_vec1,
            new_tracks: producer,
        },
        Mixer {
            tracks,
            mix_point: MixPoint::new(max_buffer_size),

            new_vec: new_vec2,
            new_tracks: consumer,
        },
    )
}

pub struct Mixer {
    tracks: Vec<MixerTrack>,
    mix_point: MixPoint,

    new_vec: Arc<AtomicOptionBox<Vec<MixerTrack>>>,
    new_tracks: ringbuf::Consumer<MixerTrack>,
}
impl Mixer {
    pub fn poll(&mut self) {
        let new_vec = self.new_vec.take(Ordering::SeqCst);
        if let Some(new_vec) = new_vec {
            let old_vec = mem::replace(&mut self.tracks, *new_vec);
            for track in old_vec {
                self.tracks.push(track);
            }
        }

        self.new_tracks.pop_each(
            |track| {
                self.tracks.push(track);
                true
            },
            None,
        );

        // TODO: Drop old_vec in another thread?
    }

    pub fn output(&mut self, sample_rate: u32, buffer_size: usize) -> &mut [Sample] {
        self.mix_point.reset();
        for track in self.tracks.iter_mut() {
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
    pub tracks: Vec<MixerTrackInterface>,

    cap: usize,
    new_vec: Arc<AtomicOptionBox<Vec<MixerTrack>>>,
    new_tracks: ringbuf::Producer<MixerTrack>,
}
impl MixerInterface {
    pub fn add_track(&mut self, max_buffer_size: usize) {
        let (track_interface, track) = new_mixer_track(max_buffer_size);
        self.tracks.push(track_interface);

        // Do the work of reallocating Mixer's vector if needed
        if self.tracks.len() > self.cap {
            self.cap *= 2;
            self.new_vec.store(
                Some(Box::new(Vec::with_capacity(self.cap))),
                Ordering::SeqCst,
            );
        }

        if let Err(_) = self.new_tracks.push(track) {
            panic!("Too many tracks added to mixer inbetween polls.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_single_track() {
        let max_buffer_size = 10;
        let (mut mi, mut m) = new_mixer(max_buffer_size);

        mi.add_track(max_buffer_size);
        m.poll();

        assert_eq!(m.tracks.len(), 2);
    }

    #[test]
    fn add_multiple_tracks() {
        let max_buffer_size = 10;
        let (mut mi, mut m) = new_mixer(max_buffer_size);

        for _ in 0..5 {
            mi.add_track(max_buffer_size);
        }
        m.poll();

        assert_eq!(m.tracks.len(), 6);
    }

    #[test]
    #[should_panic]
    fn add_too_many_tracks() {
        let max_buffer_size = 10;
        let (mut mi, mut m) = new_mixer(max_buffer_size);

        for _ in 0..11 {
            mi.add_track(max_buffer_size);
        }
        m.poll();

        assert_eq!(m.tracks.len(), 6);
    }
}
