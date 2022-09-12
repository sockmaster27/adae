mod audio_clip;
mod audio_clip_store;

use std::{
    cell::Cell,
    ops::{Add, Sub},
};

use audio_clip::AudioClip;

use crate::{
    engine::{
        traits::{Info, Source},
        Sample,
    },
    zip,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Timestamp {
    milli_beats: u64,
}
impl Timestamp {
    fn from_milli_beats(milli_beats: u64) -> Self {
        Self { milli_beats }
    }
    fn from_beats(beats: u64) -> Self {
        Self {
            milli_beats: beats * 1000,
        }
    }
    fn from_samples(samples: u128, sample_rate: u128, bpm: u128) -> Self {
        let micro_seconds = samples * 10_000 / sample_rate;
        let milli_beats = micro_seconds * bpm / (60 * 10);
        Self {
            milli_beats: milli_beats.try_into().expect("Overflow"),
        }
    }

    fn milli_beats(&self) -> u64 {
        self.milli_beats
    }
    fn beats(&self) -> u64 {
        self.milli_beats / 1000
    }
    fn samples(&self, sample_rate: u128, bpm: u128) -> u128 {
        let milli_beats: u128 = self.milli_beats.into();
        let micro_seconds = (milli_beats * 60 * 10) / bpm;
        sample_rate * micro_seconds / 10_000
    }
}
impl Add for Timestamp {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            milli_beats: self.milli_beats + rhs.milli_beats,
        }
    }
}
impl Sub for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            milli_beats: self.milli_beats - rhs.milli_beats,
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

#[derive(Debug)]
pub struct TimelineTrack {
    clips: Vec<TimelineClip>,
    relevant_clip: usize,
    position: u64,

    output_buffer: Vec<Sample>,
}
impl Source for TimelineTrack {
    fn output(&mut self, info: Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = info;

        let mut samples = 0;
        while samples < buffer_size {
            let relevant_clip = &mut self.clips[self.relevant_clip];
            let output = relevant_clip.output(Info {
                sample_rate,
                buffer_size: buffer_size - samples,
            });
            for (&mut sample, sample_out) in zip!(output, self.output_buffer[samples..].iter_mut())
            {
                *sample_out = sample;
            }
            samples += output.len();

            if output.len() < buffer_size {
                self.relevant_clip += 1;
            }
        }

        &mut self.output_buffer[..buffer_size]
    }
}

#[derive(Debug)]
pub struct Timeline {
    tracks: Vec<TimelineTrack>,
    position: u64,
}

#[cfg(test)]
mod tests {
    use std::result;

    use super::*;

    #[test]
    fn timestamp_beats_to_beats() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn timestamp_beats_to_milli_beats() {
        let ts = Timestamp::from_beats(8605);
        assert_eq!(ts.milli_beats(), 8605_000);
    }
    #[test]
    fn timestamp_milli_beats_to_beats() {
        let ts = Timestamp::from_milli_beats(8605_982);
        assert_eq!(ts.beats(), 8605);
    }
    #[test]
    fn timestamp_milli_beats_to_samples() {
        let ts = Timestamp::from_milli_beats(1_000_000);
        let result = ts.samples(40_000, 100);
        assert_eq!(result, 24_000_000);
    }
    #[test]
    fn timestamp_max_milli_beats_to_samples() {
        let ts = Timestamp::from_milli_beats(u64::MAX);
        let result = ts.samples(40_000, 100);
        assert_eq!(result, 442_721_857_769_029_238_760);
    }
}
