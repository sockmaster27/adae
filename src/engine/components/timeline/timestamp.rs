use std::ops::{Add, Sub};

const UNITS_PER_BEAT: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    /// 1 beat = 1024 beat units, making it highly divisible by powers of 2
    beat_units: u32,
}
impl Timestamp {
    /// The smallest possible Timestamp representing the very beginning (regardless of unit)
    pub fn zero() -> Self {
        Self { beat_units: 0 }
    }

    /// 1 beat = 1024 beat units
    pub fn from_beat_units(beat_units: u32) -> Self {
        Self { beat_units }
    }
    pub fn from_beats(beats: u32) -> Self {
        Self {
            beat_units: beats * UNITS_PER_BEAT,
        }
    }
    pub fn from_samples(samples: u64, sample_rate: u32, bpm_cents: u16) -> Self {
        let beat_units =
            (samples * bpm_cents as u64 * UNITS_PER_BEAT as u64) / (sample_rate as u64 * 60 * 100);
        Self {
            beat_units: beat_units.try_into().expect("Overflow"),
        }
    }

    pub fn beat_units(&self) -> u32 {
        self.beat_units
    }
    pub fn beats(&self) -> u32 {
        self.beat_units / UNITS_PER_BEAT
    }
    pub fn samples(&self, sample_rate: u32, bpm_cents: u16) -> u64 {
        (self.beat_units as u64 * sample_rate as u64 * 60 * 100)
            / (bpm_cents as u64 * UNITS_PER_BEAT as u64)
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
    fn zero() {
        let ts = Timestamp::zero();
        assert_eq!(ts.beat_units(), 0);
        assert_eq!(ts.beats(), 0);
        assert_eq!(ts.samples(40_000, 100), 0);
    }

    #[test]
    fn beats_to_beats() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn beats_to_beat_units() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.beat_units(), 8_811_520);
    }
    #[test]
    fn beat_units_to_beats() {
        let ts = Timestamp::from_beat_units(8_812_520);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn beat_units_to_samples() {
        let ts = Timestamp::from_beat_units(1_024_000);
        let result = ts.samples(40_000, 100_00);
        assert_eq!(result, 24_000_000);
    }
    #[test]
    fn max_milli_beats_to_samples() {
        let ts = Timestamp::from_beat_units(u32::MAX);
        let result = ts.samples(40_000, 100_00);
        assert_eq!(result, (u32::MAX as u64 * 40_000 * 60) / (100 * 1024));
    }
}
