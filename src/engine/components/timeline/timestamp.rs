use std::ops::{Add, Sub};

const UNITS_PER_BEAT: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// 1 beat = 1024 beat units, making it highly divisible by powers of 2
    beat_units: u64,
}
impl Timestamp {
    /// 1 beat = 1024 beat units
    pub fn from_beat_units(beat_units: u64) -> Self {
        Self { beat_units }
    }
    pub fn from_beats(beats: u64) -> Self {
        Self {
            beat_units: beats * UNITS_PER_BEAT,
        }
    }
    pub fn from_samples(samples: u128, sample_rate: u128, bpm: u128) -> Self {
        let beat_units = (samples * bpm * UNITS_PER_BEAT as u128) / (sample_rate * 60);
        Self {
            beat_units: beat_units.try_into().expect("Overflow"),
        }
    }

    pub fn beat_units(&self) -> u64 {
        self.beat_units
    }
    pub fn beats(&self) -> u64 {
        self.beat_units / UNITS_PER_BEAT
    }
    pub fn samples(&self, sample_rate: u128, bpm: u128) -> u128 {
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
