use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::ops::{Add, Mul, Sub};

const UNITS_PER_BEAT: u32 = 1024;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// 1 beat = 1024 beat units, making it highly divisible by powers of 2
    beat_units: u32,
}
impl Timestamp {
    /// The smallest possible timestamp representing the very beginning (regardless of unit)
    pub const fn zero() -> Self {
        Self { beat_units: 0 }
    }
    /// The largest representable timestamp, convenient for comparisons.
    /// Converting this to anything other than beat units may overflow.
    pub const fn infinity() -> Self {
        Self {
            beat_units: u32::MAX,
        }
    }

    /// 1 beat = 1024 beat units
    pub const fn from_beat_units(beat_units: u32) -> Self {
        Self { beat_units }
    }
    pub const fn from_beats(beats: u32) -> Self {
        Self {
            beat_units: beats * UNITS_PER_BEAT,
        }
    }
    /// Converts a number of samples to a timestamp.
    ///
    /// This rounds down. If this is undesirable, see `Timestamp::from_samples_ceil`.
    pub const fn from_samples(samples: usize, sample_rate: u32, bpm_cents: u16) -> Self {
        let beat_units = (samples * bpm_cents as usize * UNITS_PER_BEAT as usize)
            / (sample_rate as usize * 60 * 100);
        Self {
            beat_units: beat_units as u32,
        }
    }
    /// Like [`Timestamp::from_samples`], but rounds up instead of down.
    pub const fn from_samples_ceil(samples: u64, sample_rate: u32, bpm_cents: u16) -> Self {
        const fn div_ceil(a: u64, b: u64) -> u64 {
            let d = a / b;
            let r = a % b;
            if r != 0 {
                d + 1
            } else {
                d
            }
        }

        let beat_units = div_ceil(
            samples * bpm_cents as u64 * UNITS_PER_BEAT as u64,
            sample_rate as u64 * 60 * 100,
        );
        Self {
            beat_units: beat_units as u32,
        }
    }

    /// 1 beat = 1024 beat units
    pub const fn beat_units(&self) -> u32 {
        self.beat_units
    }
    pub const fn beats(&self) -> u32 {
        self.beat_units / UNITS_PER_BEAT
    }
    pub const fn samples(&self, sample_rate: u32, bpm_cents: u16) -> usize {
        (self.beat_units as usize * sample_rate as usize * 60 * 100)
            / (bpm_cents as usize * UNITS_PER_BEAT as usize)
    }

    pub fn checked_add(&self, rhs: Self) -> Option<Self> {
        self.beat_units
            .checked_add(rhs.beat_units)
            .map(Self::from_beat_units)
    }
    pub fn checked_sub(&self, rhs: Self) -> Option<Self> {
        self.beat_units
            .checked_sub(rhs.beat_units)
            .map(Self::from_beat_units)
    }
}
impl Debug for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.beat_units == u32::MAX {
            return f.write_str("Timestamp::infinity()");
        }

        f.debug_struct("Timestamp")
            .field("beat_units", &self.beat_units)
            .finish()
    }
}
impl Add for Timestamp {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        #[cfg(debug_assertions)]
        if self.checked_add(rhs).is_none() {
            panic!("Timestamp addition with overflow: {self:?} + {rhs:?}");
        }

        Self {
            beat_units: self.beat_units + rhs.beat_units,
        }
    }
}
impl Sub for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        #[cfg(debug_assertions)]
        if self.checked_sub(rhs).is_none() {
            panic!("Timestamp subtraction with overflow: {self:?} - {rhs:?}");
        }

        Self {
            beat_units: self.beat_units - rhs.beat_units,
        }
    }
}
impl Mul<u32> for Timestamp {
    type Output = Timestamp;
    fn mul(self, rhs: u32) -> Self::Output {
        Timestamp {
            beat_units: self.beat_units * rhs,
        }
    }
}
impl Mul<Timestamp> for u32 {
    type Output = Timestamp;
    fn mul(self, rhs: Timestamp) -> Self::Output {
        Timestamp {
            beat_units: self * rhs.beat_units,
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
        assert_eq!(result, (u32::MAX as usize * 40_000 * 60) / (100 * 1024));
    }

    #[test]
    fn add() {
        let a = Timestamp::from_beat_units(42);
        let b = Timestamp::from_beat_units(43);
        let result = a + b;
        assert_eq!(result.beat_units(), 85);
    }
    #[test]
    fn sub() {
        let a = Timestamp::from_beat_units(43);
        let b = Timestamp::from_beat_units(42);
        let result = a - b;
        assert_eq!(result.beat_units(), 1);
    }
    #[test]
    fn mul() {
        let ts = Timestamp::from_beat_units(42);
        assert_eq!((ts * 2).beat_units(), 84);
        assert_eq!((2 * ts).beat_units(), 84);
    }
}
