mod audio_clip;
mod audio_clip_store;
mod track;

pub use track::{timeline_track, TimelineTrack};

use std::{
    cell::Cell,
    ops::{Add, Sub},
    sync::{atomic::AtomicU64, Arc},
};

use crate::engine::{traits::Info, Sample};
use audio_clip::AudioClip;

const UNITS_PER_BEAT: u64 = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Timestamp {
    /// 1 beat = 1024 beat units, making it highly divisible by powers of 2
    beat_units: u64,
}
impl Timestamp {
    /// 1 beat = 1024 beat units
    fn from_beat_units(beat_units: u64) -> Self {
        Self { beat_units }
    }
    fn from_beats(beats: u64) -> Self {
        Self {
            beat_units: beats * UNITS_PER_BEAT,
        }
    }
    fn from_samples(samples: u128, sample_rate: u128, bpm: u128) -> Self {
        let beat_units = (samples * bpm * UNITS_PER_BEAT as u128) / (sample_rate * 60);
        Self {
            beat_units: beat_units.try_into().expect("Overflow"),
        }
    }

    fn beat_units(&self) -> u64 {
        self.beat_units
    }
    fn beats(&self) -> u64 {
        self.beat_units / UNITS_PER_BEAT
    }
    fn samples(&self, sample_rate: u128, bpm: u128) -> u128 {
        (self.beat_units as u128 * sample_rate * 60) / (bpm * UNITS_PER_BEAT as u128)
    }
}
impl Add for Timestamp {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            beat_units: self.beat_units + rhs.beat_units,
        }
    }
}
impl Sub for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            beat_units: self.beat_units - rhs.beat_units,
        }
    }
}

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
    fn new(start: Timestamp, audio_clip: AudioClip) -> Self {
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

    fn output(&mut self, info: Info) -> &mut [Sample] {
        self.inner.output(info)
    }
}

pub fn timeline() -> (Timeline, TimelineProcessor) {
    let sync_position1 = Arc::new(AtomicU64::new(0));
    let sync_position2 = Arc::clone(&sync_position1);

    (
        Timeline {
            sync_position: sync_position1,
        },
        TimelineProcessor {
            sync_position: sync_position2,
            position: Cell::new(0),
        },
    )
}

#[derive(Debug)]
pub struct Timeline {
    sync_position: Arc<AtomicU64>,
}
pub struct TimelineProcessor {
    sync_position: Arc<AtomicU64>,
    position: Cell<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_beats_to_beats() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn timestamp_beats_to_beat_units() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.beat_units(), 8_811_520);
    }
    #[test]
    fn timestamp_beat_units_to_beats() {
        let ts = Timestamp::from_beat_units(8_812_520);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn timestamp_beat_units_to_samples() {
        let ts = Timestamp::from_beat_units(1_024_000);
        let result = ts.samples(40_000, 100);
        assert_eq!(result, 24_000_000);
    }
    #[test]
    fn timestamp_max_milli_beats_to_samples() {
        let ts = Timestamp::from_beat_units(u64::MAX);
        let result = ts.samples(40_000, 100);
        assert_eq!(result, 432_345_564_227_567_615_976);
    }
}
