use super::TimelineClip;
use crate::engine::traits::{Component, Info, Source};
use crate::engine::{Sample, CHANNELS};
use crate::zip;

pub fn timeline_track(max_buffer_size: usize) -> (TimelineTrack, TimelineTrackProcessor) {
    (
        TimelineTrack,
        TimelineTrackProcessor {
            clips: Vec::new(),
            relevant_clip: None,
            output_buffer: Vec::with_capacity(max_buffer_size * CHANNELS),
        },
    )
}

pub struct TimelineTrack;

#[derive(Debug)]
pub struct TimelineTrackProcessor {
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
                let output = relevant_clip.output(Info {
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
