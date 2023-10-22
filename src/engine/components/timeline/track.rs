use intrusive_collections::rbtree::CursorOwning;
use intrusive_collections::{Bound, RBTree};
use serde::{Deserialize, Serialize};
use std::cell::RefMut;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::audio_clip::{AudioClip, AudioClipKey, AudioClipState};
use super::AudioClipProcessor;
use crate::engine::components::track::MixerTrackKey;
use crate::engine::info::Info;
use crate::engine::utils::dropper::{self, DBox};
use crate::engine::utils::rbtree_node::{TreeNode, TreeNodeAdapter};
use crate::engine::{Sample, CHANNELS};
use crate::Timestamp;

pub type TimelineTrackKey = u32;

/// A mirror for the state of a `TimelineTrackProcessor`.
/// Unlike the convention, this does not do any synchronization with the `TimelineTrackProcessor`.
pub struct TimelineTrack {
    pub clips: HashMap<AudioClipKey, AudioClip>,
    pub output_track: MixerTrackKey,
}
impl TimelineTrack {
    pub fn new(output: MixerTrackKey) -> Self {
        TimelineTrack {
            clips: HashMap::new(),
            output_track: output,
        }
    }
}

pub struct TimelineTrackProcessor {
    position: Arc<AtomicU64>,
    sample_rate: u32,
    bpm_cents: u16,

    /// Optional because it needs to be swapped out in jump_to.
    /// Should always be safely unwrappable.
    relevant_clip: Option<CursorOwning<TreeNodeAdapter<AudioClipProcessor>>>,

    output_track: MixerTrackKey,
}
impl TimelineTrackProcessor {
    pub fn new(
        output: MixerTrackKey,
        position: Arc<AtomicU64>,
        sample_rate: u32,
        bpm_cents: u16,
    ) -> Self {
        let tree = RBTree::new(TreeNodeAdapter::new());
        let relevant_clip = Some(tree.cursor_owning());

        TimelineTrackProcessor {
            position,
            sample_rate,
            bpm_cents,

            relevant_clip,

            output_track: output,
        }
    }

    pub fn output_track(&self) -> MixerTrackKey {
        self.output_track
    }

    pub fn insert_clip(&mut self, clip: Box<TreeNode<AudioClipProcessor>>) {
        self.relevant_clip
            .as_mut()
            .unwrap()
            .with_cursor_mut(|cursor| {
                let mut clip_ref: RefMut<AudioClipProcessor> = (*clip).borrow_mut();

                let pos_samples = self.position.load(Ordering::Relaxed);
                let position =
                    Timestamp::from_samples(pos_samples, self.sample_rate, self.bpm_cents);
                let clip_end = clip_ref.end(self.sample_rate, self.bpm_cents);
                let next = cursor.get();

                let is_more_relevant = match next {
                    Some(next) => position < clip_end && clip_end <= next.borrow().start,
                    None => position < clip_end,
                };

                if clip_ref.start <= position && position < clip_end {
                    clip_ref
                        .jump_to(position, self.sample_rate, self.bpm_cents)
                        .unwrap();
                }

                drop(clip_ref);
                cursor.insert(clip);
                if is_more_relevant {
                    cursor.move_prev();
                }
            })
    }

    pub fn delete_clip(&mut self, clip_start: Timestamp) {
        let mut tree = self.relevant_clip.take().unwrap().into_inner();

        let el = tree
            .find_mut(&clip_start)
            .remove()
            .expect("Attempted to delete non-existing clip");
        dropper::send(el);

        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, self.sample_rate, self.bpm_cents);
        self.relevant_clip = Some(tree.upper_bound_owning(Bound::Included(&position)));
    }
    pub fn delete_clips(&mut self, clip_starts: DBox<Vec<Timestamp>>) {
        let mut tree = self.relevant_clip.take().unwrap().into_inner();

        for clip_start in clip_starts.iter() {
            let el = tree
                .find_mut(clip_start)
                .remove()
                .expect("Attempted to delete non-existing clip");
            dropper::send(el);
        }

        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, self.sample_rate, self.bpm_cents);
        self.relevant_clip = Some(tree.upper_bound_owning(Bound::Included(&position)));
    }

    pub fn crop_clip_end(&mut self, clip_start: Timestamp, new_length: Timestamp) {
        self.with_clip(clip_start, |clip| {
            clip.length = Some(new_length);
        });
    }

    /// Jump to the global position.
    /// Should be called when the global position changes.
    pub fn jump(&mut self) {
        let sample_rate = self.sample_rate;
        let bpm_cents = self.bpm_cents;

        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, sample_rate, self.bpm_cents);

        self.with_relevant_clip(|old_clip_opt| {
            if let Some(old_clip) = old_clip_opt {
                old_clip.reset(sample_rate);
            }
        });

        self.update_relevant_clip();

        self.with_relevant_clip(|new_clip_opt| {
            if let Some(new_clip) = new_clip_opt {
                if new_clip.start <= position {
                    new_clip.jump_to(position, sample_rate, bpm_cents).unwrap();
                }
            }
        });
    }

    pub fn output(&mut self, info: &Info, buffer: &mut [Sample]) {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        self.relevant_clip
            .as_mut()
            .unwrap()
            .with_cursor_mut(|cursor| {
                let mut progress = 0;

                while progress < buffer_size {
                    let should_move;
                    match cursor.get() {
                        Some(clip_cell) => {
                            let position = self.position.load(Ordering::Relaxed);
                            let mut clip = clip_cell.borrow_mut();

                            // Pad start with zero
                            let clip_start = clip.start.samples(sample_rate, self.bpm_cents);
                            if position + (progress as u64) < clip_start {
                                let end_zeroes = min((clip_start - position) as usize, buffer_size);
                                buffer[progress * CHANNELS..end_zeroes * CHANNELS].fill(0.0);
                                progress = end_zeroes;
                            }

                            // Fill with content
                            let requested_buffer = buffer_size - progress;
                            let output = clip.output(
                                self.bpm_cents,
                                &Info {
                                    sample_rate,
                                    buffer_size: requested_buffer,
                                },
                            );
                            buffer[progress * CHANNELS..progress * CHANNELS + output.len()]
                                .copy_from_slice(output);
                            progress += output.len() / CHANNELS;

                            // Determine if we should move on to next clip
                            should_move = output.len() < requested_buffer;
                            if should_move {
                                clip.reset(sample_rate);
                            }
                        }

                        None => {
                            // No more clips, pad with zero
                            buffer[progress * CHANNELS..buffer_size * CHANNELS].fill(0.0);
                            break;
                        }
                    }

                    // Needs to be out here to access &mut cursor
                    if should_move {
                        cursor.move_next();
                    }
                }
            });
    }

    fn with_relevant_clip(&mut self, f: impl FnOnce(Option<&mut AudioClipProcessor>)) {
        self.relevant_clip
            .as_mut()
            .unwrap()
            .with_cursor_mut(|cursor| match cursor.get() {
                // .map() doesn't work because of the RefMut lifetime :(
                Some(clip_cell) => {
                    let clip = &mut *clip_cell.borrow_mut();
                    f(Some(clip));
                }
                None => f(None),
            });
    }

    /// Run a function on the clip at `clip_start`, and reset `self.relevant_clip` via `self.find_relevant_clip()`.
    fn with_clip(&mut self, clip_start: Timestamp, f: impl FnOnce(&mut AudioClipProcessor)) {
        let tree = self.relevant_clip.take().unwrap().into_inner();
        let cursor = tree.find(&clip_start);
        let clip_cell = cursor.get().expect("Attempted to access non-existing clip");
        let mut clip = clip_cell.borrow_mut();

        f(&mut clip);
        drop(clip);

        // Set relevant_clip to arbitrary value before calling update_relevant_clip
        self.relevant_clip = Some(tree.cursor_owning());

        // The relevant clip should be the same as before
        self.update_relevant_clip();
    }

    /// Set the relevant clip in accordance with the current position.
    /// Requires `self.relevant_clip` to be `Some`.
    ///
    /// Note: The positions within both the old and new relevant clips are preserved.
    fn update_relevant_clip(&mut self) {
        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, self.sample_rate, self.bpm_cents);

        let tree = self
            .relevant_clip
            .take()
            .expect("self.relevant_clip is None")
            .into_inner();
        self.relevant_clip = Some(tree.upper_bound_owning(Bound::Included(&position)));

        self.relevant_clip
            .as_mut()
            .unwrap()
            .with_cursor_mut(|cursor| {
                let clip = cursor.get();
                match clip {
                    None => cursor.move_next(),
                    Some(clip) => {
                        let clip_end = clip.borrow().end(self.sample_rate, self.bpm_cents);
                        if clip_end <= position {
                            cursor.move_next();
                        }
                    }
                }
            });
    }
}
impl Debug for TimelineTrackProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimelineTrack")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TimelineTrackState {
    pub key: TimelineTrackKey,
    pub clips: Vec<AudioClipState>,
    pub output_track: MixerTrackKey,
}
impl PartialEq for TimelineTrackState {
    fn eq(&self, other: &Self) -> bool {
        let self_set: HashSet<_> = HashSet::from_iter(self.clips.iter());
        let other_set: HashSet<_> = HashSet::from_iter(other.clips.iter());

        debug_assert_eq!(
            self_set.len(),
            self.clips.len(),
            "Duplicate clips in TimelineTrackState: {:?}",
            self.clips
        );
        debug_assert_eq!(
            other_set.len(),
            other.clips.len(),
            "Duplicate clips in TimelineTrackState: {:?}",
            other.clips
        );

        self.key == other.key
    }
}
impl Eq for TimelineTrackState {}
impl Hash for TimelineTrackState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

#[cfg(test)]
mod tests {

    use crate::engine::{
        components::{audio_clip_reader::AudioClipReader, stored_audio_clip::StoredAudioClip},
        utils::test_file_path,
    };

    use super::*;

    const SAMPLE_RATE: u32 = 40_960;
    const BPM_CENTS: u16 = 120_00;
    /// Samples per Beat Unit
    const SBU: usize = Timestamp::from_beat_units(1).samples(SAMPLE_RATE, BPM_CENTS) as usize;

    fn clip(
        start_beat_units: u32,
        length_beat_units: Option<u32>,
        max_buffer_size: usize,
    ) -> Box<TreeNode<AudioClipProcessor>> {
        thread_local! {
            static AC: Arc<StoredAudioClip> = Arc::new(StoredAudioClip::import(0, &test_file_path("44100 16-bit.wav")).unwrap());
        }

        AC.with(|ac| {
            Box::new(TreeNode::new(AudioClipProcessor::new(
                Timestamp::from_beat_units(start_beat_units),
                length_beat_units.map(Timestamp::from_beat_units),
                0,
                AudioClipReader::new(Arc::clone(ac), max_buffer_size, 48_000),
            )))
        })
    }

    #[test]
    fn insert() {
        let mut t =
            TimelineTrackProcessor::new(0, Arc::new(AtomicU64::new(0)), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(3, Some(1), 100);
        let c2 = clip(1, Some(2), 100);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);
        }}

        // c2 should be inserted before c1
        t.relevant_clip.unwrap().with_cursor_mut(|cur| {
            let clip = cur.get().unwrap().borrow();
            let length = clip.length.unwrap();
            assert_eq!(length.beat_units(), 2);
        });
    }

    #[test]
    fn output() {
        const BUFFER_SIZE: usize = SBU;
        let info = Info {
            sample_rate: SAMPLE_RATE,
            buffer_size: BUFFER_SIZE,
        };
        let pos = Arc::new(AtomicU64::new(0));
        let mut t = TimelineTrackProcessor::new(0, Arc::clone(&pos), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(1, Some(1), 3 * SBU);
        let c2 = clip(3, Some(2), 3 * SBU);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            // c1
            t.relevant_clip.as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get().unwrap().borrow();
                let length = clip.length.unwrap();
                assert_eq!(length.beat_units(), 1);
            });

            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_ne!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            // c2
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get().unwrap().borrow();
                let length = clip.length.unwrap();
                assert_eq!(length.beat_units(), 2);
            });

            let mut out = [0.0; 3 * SBU * CHANNELS];
            t.output(&Info {
                sample_rate: SAMPLE_RATE,
                buffer_size: 3 * SBU,
            }, &mut out[..]);
            for &s in &out[..CHANNELS * (2 * SBU)] {
                assert_ne!(s, 0.0);
            }
            for &s in &out[CHANNELS * (2 * SBU)..] {
                assert_eq!(s, 0.0);
            }

            // end
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
        }}
    }

    #[test]
    fn output_many() {
        const BUFFER_SIZE: usize = 10_000_000;
        const SBUC: usize = SBU * CHANNELS;
        let mut out = vec![0.0; BUFFER_SIZE * CHANNELS];

        let mut t =
            TimelineTrackProcessor::new(0, Arc::new(AtomicU64::new(0)), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(0, Some(1), BUFFER_SIZE);
        let c2 = clip(2, Some(1), BUFFER_SIZE);
        let c3 = clip(4, Some(1), BUFFER_SIZE);
        let c4 = clip(6, None, BUFFER_SIZE);
        let c4_end = c4.borrow().end(SAMPLE_RATE, BPM_CENTS).beat_units() as usize;
        let c5 = clip((c4_end as u32) + 1, Some(1), BUFFER_SIZE);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);
            t.insert_clip(c3);
            t.insert_clip(c4);
            t.insert_clip(c5);

            // Output everything in one go
            t.output(&Info {
                sample_rate: SAMPLE_RATE,
                buffer_size: (c4_end + 3) * SBUC,
            }, &mut out[..]);

            // c1
            for &s in &out[..SBUC] {
                assert_ne!(s, 0.0);
            }

            // Nothing
            for &s in &out[SBUC..2 * SBUC] {
                assert_eq!(s, 0.0);
            }

            // c2
            for &s in &out[2 * SBUC..3 * SBUC] {
                assert_ne!(s, 0.0);
            }

            // Nothing
            for &s in &out[3 * SBUC..4 * SBUC] {
                assert_eq!(s, 0.0);
            }

            // c3
            for &s in &out[4 * SBUC..5 * SBUC] {
                assert_ne!(s, 0.0);
            }

            // Nothing
            for &s in &out[5 * SBUC..6 * SBUC] {
                assert_eq!(s, 0.0);
            }

            // c4
            for &s in &out[6 * SBUC..8 * SBUC] {
                assert_ne!(s, 0.0);
            }
            for &s in &out[(c4_end - 3) * SBUC..(c4_end - 1) * SBUC] {
                assert_ne!(s, 0.0);
            }
            // c4 ends somewhere inbetween (c4_end - 1) and c4_end

            // Nothing
            for &s in &out[c4_end * SBUC..(c4_end + 1) * SBUC] {
                assert_eq!(s, 0.0);
            }

            // c5
            for &s in &out[(c4_end + 1) * SBUC..(c4_end + 2) * SBUC] {
                assert_ne!(s, 0.0);
            }

            // Nothing
            for &s in &out[(c4_end + 2) * SBUC..(c4_end + 3) * SBUC] {
                assert_eq!(s, 0.0);
            }
        }}
    }

    #[test]
    fn jump() {
        const BUFFER_SIZE: usize = SBU;
        let info = Info {
            sample_rate: SAMPLE_RATE,
            buffer_size: BUFFER_SIZE,
        };
        let pos = Arc::new(AtomicU64::new(0));
        let mut t = TimelineTrackProcessor::new(0, Arc::clone(&pos), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(1, Some(1), 100);
        let c2 = clip(3, Some(2), 100);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);

            pos.store(2 * SBU as u64, Ordering::Relaxed);
            t.jump();

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            // c2
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get().unwrap().borrow();
                let length = clip.length.unwrap();
                assert_eq!(length.beat_units(), 2);
            });

            let mut out = [0.0; 3 * SBU * CHANNELS];
            t.output(&Info {
                sample_rate: SAMPLE_RATE,
                buffer_size: 3 * SBU,
            }, &mut out[..]);
            for &s in &out[..CHANNELS * (2 * SBU)] {
                assert_ne!(s, 0.0);
            }
            for &s in &out[CHANNELS * (2 * SBU)..] {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(3 * SBU as u64, Ordering::Relaxed);

            // end
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            t.relevant_clip
            .as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            pos.store(0, Ordering::Relaxed);
            t.jump();

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);

            // c1
            t.relevant_clip.as_mut().unwrap().with_cursor_mut(|cur| {
                let clip = cur.get().unwrap().borrow();
                let length = clip.length.unwrap();
                assert_eq!(length.beat_units(), 1);
            });

            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_ne!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size as u64, Ordering::Relaxed);
        }}
    }
}
