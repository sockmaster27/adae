use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use super::timestamp::Timestamp;
use super::{AudioClip, TimelineClip, TimelineClipKey};
use crate::engine::components::audio_clip::AudioClipKey;
use crate::engine::components::event_queue::EventReceiver;
use crate::engine::traits::{Component, Info, Source};
use crate::engine::utils::key_generator::OverflowError;
use crate::engine::{Sample, CHANNELS};
use crate::zip;

pub type TimelineTrackKey = u32;

pub fn timeline_track(
    key: TimelineTrackKey,
    position: Arc<AtomicU64>,
    max_buffer_size: usize,
) -> (TimelineTrack, TimelineTrackProcessor) {
    (
        TimelineTrack { key },
        TimelineTrackProcessor {
            position,

            clips: Vec::new(),
            relevant_clip: None,

            output_buffer: Vec::with_capacity(max_buffer_size * CHANNELS),
        },
    )
}

#[derive(Debug)]
pub struct TimelineTrack {
    key: TimelineTrackKey,
}
impl TimelineTrack {
    pub fn key(&self) -> TimelineTrackKey {
        self.key
    }

    pub fn add_clip(
        &mut self,
        clip: AudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<TimelineClipKey, OverflowError> {
        todo!()
    }
}

#[derive(Debug)]
pub struct TimelineTrackProcessor {
    position: Arc<AtomicU64>,

    clips: Vec<TimelineClip>,
    relevant_clip: Option<usize>,

    output_buffer: Vec<Sample>,
}
impl Component for TimelineTrackProcessor {}
impl Source for TimelineTrackProcessor {
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
