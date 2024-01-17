use intrusive_collections::rbtree::CursorOwning;
use intrusive_collections::{Bound, RBTree};
use serde::{Deserialize, Serialize};
use std::cell::RefMut;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::audio_clip::{AudioClip, AudioClipKey, AudioClipState};
use super::AudioClipProcessor;
use crate::engine::components::track::MixerTrackKey;
use crate::engine::info::Info;
use crate::engine::utils::dropper;
use crate::engine::utils::key_generator::Key;
use crate::engine::utils::rbtree_node::{TreeNode, TreeNodeAdapter};
use crate::engine::{Sample, CHANNELS};
use crate::Timestamp;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimelineTrackKey(u32);
impl Key for TimelineTrackKey {
    type Id = u32;
    fn new(id: Self::Id) -> Self {
        Self(id)
    }
    fn id(&self) -> Self::Id {
        self.0
    }
}

type TimelineTree = RBTree<TreeNodeAdapter<AudioClipProcessor>>;
type TimelineCursor = CursorOwning<TreeNodeAdapter<AudioClipProcessor>>;

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
    position: Arc<AtomicUsize>,
    sample_rate: u32,
    bpm_cents: u16,

    /// The clip that is either currently playing, or will be played next.
    /// If this is the null pointer then the track is past the last clip.
    ///
    /// Optional to allow temporary access to the tree.
    /// Unless expicitly stated, all methods expect this to be `Some`.
    ///
    /// All clips in this tree should be reset when the the position encounters them, not when it leaves them.
    relevant_clip: Option<TimelineCursor>,

    output_track: MixerTrackKey,
}
impl TimelineTrackProcessor {
    pub fn new(
        output: MixerTrackKey,
        position: Arc<AtomicUsize>,
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

    pub fn move_clip(&mut self, old_start: Timestamp, new_start: Timestamp) {
        let sample_rate = self.sample_rate;
        let bpm_cents = self.bpm_cents;
        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, sample_rate, bpm_cents);

        let new_end = self.with_clip_moving(old_start, |clip| {
            clip.start = new_start;
            clip.end(sample_rate, bpm_cents)
        });

        let relevant_start = self.map_relevant_clip_not_moving(|clip| clip.start);
        let was_relevant = relevant_start == Some(new_start);

        let is_relevant = if was_relevant {
            let cursor = self.relevant_clip.as_mut().unwrap().as_cursor();
            let next_clip_opt = cursor.peek_next().get();
            let next_starts_before = match next_clip_opt {
                Some(next) => next.borrow().start < new_start,
                None => false,
            };
            let no_longer_relevant = new_end <= position || next_starts_before;
            if no_longer_relevant {
                self.update_relevant_clip(position);
            }

            !no_longer_relevant
        } else {
            // The clip has become relevant if it's like this:
            //
            //  position         Relevant clip or None
            //     ↓                     ↓
            //     |   ...moved ]   [ relevant ]
            //
            let has_become_relevant = position < new_end
                && relevant_start.map_or(true, |relevant_start| new_end <= relevant_start);
            if has_become_relevant {
                self.relevant_clip
                    .as_mut()
                    .unwrap()
                    .with_cursor_mut(|cursor| cursor.move_prev());
                self.map_relevant_clip_not_moving(|clip| {
                    clip.jump_to(position, sample_rate, bpm_cents).unwrap()
                });
            }

            has_become_relevant
        };

        if is_relevant {
            self.map_relevant_clip_not_moving(|clip| {
                clip.jump_to(position, sample_rate, bpm_cents).unwrap()
            });
        }
    }

    pub fn crop_clip_start(
        &mut self,
        old_start: Timestamp,
        new_start: Timestamp,
        new_length: Timestamp,
        new_start_offset: usize,
    ) {
        let sample_rate = self.sample_rate;
        let bpm_cents = self.bpm_cents;
        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, sample_rate, bpm_cents);

        self.with_clip_not_moving(old_start, |clip| {
            // While clip.start is the key, changing it will not change the position in the tree if no clips can ever be in an overlapping state.
            clip.start = new_start;
            clip.length = Some(new_length);
            clip.start_offset = new_start_offset;
        });

        // TODO: only jump when start crosses position
        self.with_relevant_clip_not_moving(|relevant_clip| {
            if let Some(relevant_clip) = relevant_clip {
                let relevant_was_cropped = relevant_clip.start == new_start;
                let was_upcoming = position <= old_start;
                let is_upcoming = position <= new_start;
                if relevant_was_cropped && (was_upcoming || is_upcoming) {
                    relevant_clip
                        .jump_to(position, sample_rate, bpm_cents)
                        .unwrap();
                }
            }
        });
    }

    pub fn crop_clip_end(&mut self, clip_start: Timestamp, new_length: Timestamp) {
        let sample_rate = self.sample_rate;
        let bpm_cents = self.bpm_cents;
        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, sample_rate, bpm_cents);

        let (start, old_end, new_end) = self.with_clip_not_moving(clip_start, |clip| {
            let old_end = clip.end(sample_rate, bpm_cents);
            clip.length = Some(new_length);
            (clip.start, old_end, clip.start + new_length)
        });

        let move_prev = start <= position && old_end < position && position <= new_end;
        let move_next = start <= position && new_end < position && position <= old_end;
        let should_move = move_prev || move_next;

        if should_move {
            if let Some(relevant_clip) = self.relevant_clip.as_mut() {
                relevant_clip.with_cursor_mut(|cursor| {
                    if move_prev {
                        cursor.move_prev();

                        if let Some(clip_cell) = cursor.get() {
                            let mut clip = clip_cell.borrow_mut();
                            clip.jump_to(position, sample_rate, bpm_cents).unwrap();
                        }
                    }
                    if move_next {
                        cursor.move_next();
                    }
                });
            }
        }
    }

    /// Jump to the global position.
    /// Should be called when the global position changes.
    pub fn jump(&mut self) {
        let pos_samples = self.position.load(Ordering::Relaxed);
        let position = Timestamp::from_samples(pos_samples, self.sample_rate, self.bpm_cents);

        self.update_relevant_clip(position);

        let sample_rate = self.sample_rate;
        let bpm_cents = self.bpm_cents;

        self.with_relevant_clip_not_moving(|clip_opt| {
            if let Some(clip) = clip_opt {
                clip.jump_to(position, sample_rate, bpm_cents).unwrap();
            }
        });
    }

    /// Update the relevant clip to point to the clip that is relevant at `position`.
    fn update_relevant_clip(&mut self, position: Timestamp) {
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
                            if position + progress < clip_start {
                                let end_zeroes = min(clip_start - position, buffer_size);
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
                        if let Some(clip_cell) = cursor.get() {
                            let mut clip = clip_cell.borrow_mut();
                            clip.reset(sample_rate);
                        }
                    }
                }
            });
    }

    /// Run a function on the (optional) relevant clip, which must not alter the ordering of the clips.
    fn with_relevant_clip_not_moving<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(Option<&mut AudioClipProcessor>) -> R,
    {
        self.relevant_clip
            .as_mut()
            .unwrap()
            .with_cursor_mut(|cursor| match cursor.get() {
                // .map() doesn't work because of the RefMut lifetime :(
                Some(clip_cell) => {
                    let clip = &mut *clip_cell.borrow_mut();
                    f(Some(clip))
                }
                None => f(None),
            })
    }

    /// Run a function on the relevant clip if one exists, which must not alter the ordering of the clips.
    fn map_relevant_clip_not_moving<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut AudioClipProcessor) -> R,
    {
        self.with_relevant_clip_not_moving(|clip_opt| clip_opt.map(f))
    }

    /// Run a function on the clip at `clip_start`, which must not alter the ordering of the clips.
    ///
    /// This will _not_ update what the relevant clip is pointing to, so if the clip is moved or cropped the cursor might not point at the correct clip.
    fn with_clip_not_moving<F, R>(&mut self, clip_start: Timestamp, f: F) -> R
    where
        F: FnOnce(&mut AudioClipProcessor) -> R,
    {
        // Safety:
        // - We don't remove any clip from the tree
        // - f cannot remove any clip from the tree
        unsafe {
            self.with_tree(|tree| {
                let cursor = tree.find(&clip_start);
                let clip_cell = cursor.get().expect("Attempted to access non-existing clip");
                let mut clip = clip_cell.borrow_mut();

                f(&mut clip)
            })
        }
    }

    /// Run a function on the clip at `clip_start`, which is free to alter the ordering of the clips.
    ///
    /// This will _not_ update what the relevant clip is pointing to, so if the clip is moved or cropped the cursor might not point at the correct clip.
    fn with_clip_moving<F, R>(&mut self, clip_start: Timestamp, f: F) -> R
    where
        F: FnOnce(&mut AudioClipProcessor) -> R,
    {
        // Safety:
        // - We don't remove any clip from the tree
        // - f cannot remove any clip from the tree
        unsafe {
            self.with_tree(|tree| {
                let clip_cell = tree
                    .find_mut(&clip_start)
                    .remove()
                    .expect("Attempted to access non-existing clip");
                let mut clip = clip_cell.borrow_mut();

                let res = f(&mut clip);

                drop(clip);
                tree.insert(clip_cell);

                res
            })
        }
    }

    /// Run a function on the tree, which must not remove the relevant clip from it.
    ///
    /// # Safety
    /// The relevant clip must not be removed from the tree.
    unsafe fn with_tree<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut TimelineTree) -> R,
    {
        // Save the cursor in the form of a raw pointer,
        // taking advantage of the fact that it must remain valid,
        // avoiding the overhead of searching for the relevant clip again.
        let relevant_clip = self.relevant_clip.take().unwrap();
        let before_ptr = relevant_clip.as_cursor().get().map(|r| r as *const _);
        let mut tree = relevant_clip.into_inner();

        let res = f(&mut tree);

        self.relevant_clip = match before_ptr {
            Some(ptr) => {
                // Safety:
                // - before_ptr is a pointer to the relevant clip, which is a node in the tree
                // - f must not remove the relevant clip from the tree
                unsafe { Some(tree.cursor_owning_from_ptr(ptr)) }
            }
            None => Some(tree.cursor_owning()),
        };

        res
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

    use crate::{
        engine::{
            components::{audio_clip_reader::AudioClipReader, stored_audio_clip::StoredAudioClip},
            utils::test_file_path,
        },
        StoredAudioClipKey,
    };

    use super::*;

    const SAMPLE_RATE: u32 = 40_960;
    const BPM_CENTS: u16 = 120_00;
    /// Samples per Beat Unit
    const SBU: usize = Timestamp::from_beat_units(1).samples(SAMPLE_RATE, BPM_CENTS);

    fn clip(
        start_beat_units: u32,
        length_beat_units: Option<u32>,
        max_buffer_size: usize,
    ) -> Box<TreeNode<AudioClipProcessor>> {
        thread_local! {
            static AC: Arc<StoredAudioClip> = Arc::new(StoredAudioClip::import(StoredAudioClipKey::new(0), &test_file_path("44100 16-bit.wav")).unwrap());
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
        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::new(AtomicUsize::new(0)),
            SAMPLE_RATE,
            BPM_CENTS,
        );
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
        let pos = Arc::new(AtomicUsize::new(0));
        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::clone(&pos),
            SAMPLE_RATE,
            BPM_CENTS,
        );
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
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

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
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

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

        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::new(AtomicUsize::new(0)),
            SAMPLE_RATE,
            BPM_CENTS,
        );
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
        let pos = Arc::new(AtomicUsize::new(0));
        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::clone(&pos),
            SAMPLE_RATE,
            BPM_CENTS,
        );
        let c1 = clip(1, Some(1), 100);
        let c2 = clip(3, Some(2), 100);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);

            pos.store(2 * SBU, Ordering::Relaxed);
            t.jump();

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

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
            pos.fetch_add(3 * SBU, Ordering::Relaxed);

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
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

            pos.store(0, Ordering::Relaxed);
            t.jump();

            // Empty
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);

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
            pos.fetch_add(info.buffer_size, Ordering::Relaxed);
        }}
    }

    #[test]
    fn move_clip_into_relevance() {
        let p = Arc::new(AtomicUsize::new(0));
        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::clone(&p),
            SAMPLE_RATE,
            BPM_CENTS,
        );
        let c = clip(0, Some(1), 100);

        no_heap! {{
            t.insert_clip(c);

            // Jump to past the end of the clip
            p.store(SBU, Ordering::Relaxed);
            t.jump();

            // The clip should now no longer be the relevant clip
            t.with_relevant_clip_not_moving(|clip_opt| {
                assert!(clip_opt.is_none());
            });

            // Move the clip past the position
            t.move_clip(Timestamp::from_beat_units(0), Timestamp::from_beat_units(1));
        }}

        // The clip should now be the relevant clip
        t.with_relevant_clip_not_moving(|clip_opt| {
            assert!(clip_opt.is_some());
        });
    }

    #[test]
    fn move_clip_out_of_relevance() {
        let p = Arc::new(AtomicUsize::new(0));
        let mut t = TimelineTrackProcessor::new(
            MixerTrackKey::new(0),
            Arc::clone(&p),
            SAMPLE_RATE,
            BPM_CENTS,
        );
        let c = clip(1, Some(1), 100);

        no_heap! {{
            t.insert_clip(c);

            // Jump to past the end of the clip
            p.store(SBU, Ordering::Relaxed);
            t.jump();

            // The clip should now be the relevant clip
            t.with_relevant_clip_not_moving(|clip_opt| {
                assert!(clip_opt.is_some());
            });

            // Move the clip past the position
            t.move_clip(Timestamp::from_beat_units(1), Timestamp::from_beat_units(0));
        }}

        // The clip should now no longer be the relevant clip
        t.with_relevant_clip_not_moving(|clip_opt| {
            assert!(clip_opt.is_none());
        });
    }
}
