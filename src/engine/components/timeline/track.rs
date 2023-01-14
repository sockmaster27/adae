use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use super::TimelineClip;
use crate::engine::components::track::TrackKey;
use crate::engine::traits::{Info, Source};
use crate::engine::{Sample, CHANNELS};
use crate::zip;

pub type TimelineTrackKey = u32;

#[derive(Debug)]
pub struct TimelineTrack {
    position: Arc<AtomicU64>,

    clips: Vec<TimelineClip>,
    relevant_clip: Option<usize>,

    output_track: TrackKey,

    output_buffer: Vec<Sample>,
}
impl TimelineTrack {
    pub fn new(output: TrackKey, position: Arc<AtomicU64>, max_buffer_size: usize) -> Self {
        TimelineTrack {
            position,

            clips: Vec::with_capacity(10),
            relevant_clip: None,

            output_track: output,

            output_buffer: Vec::with_capacity(max_buffer_size * CHANNELS),
        }
    }

    pub fn output_track(&self) -> TrackKey {
        self.output_track
    }

    pub fn insert_clip(&mut self, clip: TimelineClip) {
        // Temporary prototype
        self.clips.push(clip)
    }
}
impl Source for TimelineTrack {
    fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        if let Some(mut relevant_clip_i) = self.relevant_clip {
            let mut samples = 0;
            while samples < buffer_size && relevant_clip_i < self.clips.len() {
                let relevant_clip = &mut self.clips[relevant_clip_i];
                let output = relevant_clip.output(&Info {
                    sample_rate,
                    buffer_size: buffer_size - samples,
                });
                for (&mut sample, sample_out) in
                    zip!(output, self.output_buffer[samples..].iter_mut())
                {
                    *sample_out = sample;
                }
                samples += output.len();

                if output.len() < buffer_size {
                    relevant_clip_i += 1;
                }
            }

            if relevant_clip_i >= self.clips.len() {
                self.relevant_clip = None;
            } else {
                self.relevant_clip = Some(relevant_clip_i);
            }
        }

        &mut self.output_buffer[..buffer_size]
    }
}
