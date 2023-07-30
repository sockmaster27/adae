use std::cmp::min;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use intrusive_collections::rbtree::{CursorMut, RBTreeOps};
use intrusive_collections::{Adapter, RBTree};
use ouroboros::self_referencing;

use super::TimelineClip;
use crate::engine::components::track::MixerTrackKey;
use crate::engine::info::Info;
use crate::engine::utils::rbtree_node::{TreeNode, TreeNodeAdapter};
use crate::engine::{Sample, CHANNELS};
use crate::Timestamp;

pub type TimelineTrackKey = u32;

#[self_referencing]
struct CursorOwning<A: Adapter + 'static>
where
    <A as intrusive_collections::Adapter>::LinkOps: RBTreeOps,
{
    tree: RBTree<A>,
    #[borrows(mut tree)]
    #[covariant]
    cursor: CursorMut<'this, A>,
}

// Allow sending to another thread if the ownership (represented by the <A::PointerOps as PointerOps>::Pointer owned
// pointer type) can be transferred to another thread.
unsafe impl<A: Adapter + Send> Send for CursorOwning<A>
where
    RBTree<A>: Send,
    <A as intrusive_collections::Adapter>::LinkOps: RBTreeOps,
{
}

pub struct TimelineTrack {
    position: Arc<AtomicU64>,
    sample_rate: u32,
    bpm_cents: u16,

    relevant_clip: CursorOwning<TreeNodeAdapter<TimelineClip>>,

    output_track: MixerTrackKey,
}
impl TimelineTrack {
    pub fn new(
        output: MixerTrackKey,
        position: Arc<AtomicU64>,
        sample_rate: u32,
        bpm_cents: u16,
    ) -> Self {
        TimelineTrack {
            position,
            sample_rate,
            bpm_cents,

            relevant_clip: CursorOwning::new(RBTree::new(TreeNodeAdapter::new()), |tree| {
                tree.front_mut()
            }),

            output_track: output,
        }
    }

    pub fn output_track(&self) -> MixerTrackKey {
        self.output_track
    }

    pub fn insert_clip(&mut self, clip: Box<TreeNode<TimelineClip>>) {
        self.relevant_clip.with_cursor_mut(|cursor| {
            let position = Timestamp::from_samples(
                self.position.load(Ordering::Relaxed),
                self.sample_rate,
                self.bpm_cents,
            );
            let clip_end = clip.borrow().end(self.sample_rate, self.bpm_cents);
            let next = cursor.get();

            let is_more_relevant = match next {
                Some(next) => position < clip_end && clip_end <= next.borrow().start,
                None => position < clip_end,
            };

            cursor.insert(clip);
            if is_more_relevant {
                cursor.move_prev();
            }
        })
    }

    pub fn jump(&mut self, position: Timestamp) {
        todo!("Update relevant_clip");
    }

    pub fn output(&mut self, info: &Info, buffer: &mut [Sample]) {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        self.relevant_clip.with_cursor_mut(|cursor| {
            let mut progress = 0;

            while progress < buffer_size {
                let should_move;
                match cursor.get() {
                    Some(clip) => {
                        let position = self.position.load(Ordering::Relaxed);
                        let mut clip_ref = clip.borrow_mut();

                        // Pad start with zero
                        let clip_start = clip_ref.start.samples(sample_rate, self.bpm_cents);
                        if position + (progress as u64) < clip_start {
                            let end_zeroes = min((clip_start - position) as usize, buffer_size);
                            buffer[progress * CHANNELS..end_zeroes * CHANNELS].fill(0.0);
                            progress = end_zeroes;
                        }

                        // Fill with content
                        let requested_buffer = buffer_size - progress;
                        let output = clip_ref.output(
                            self.bpm_cents,
                            &Info {
                                sample_rate,
                                buffer_size: requested_buffer,
                            },
                        );
                        buffer[progress * CHANNELS..progress * CHANNELS + output.len()]
                            .copy_from_slice(&output);
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
                }
            }
        });
    }
}
impl Debug for TimelineTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimelineTrack")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TimelineTrackState {
    pub key: TimelineTrackKey,
}

#[cfg(test)]
mod tests {

    use crate::engine::{
        components::{audio_clip::AudioClip, audio_clip_reader::AudioClipReader},
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
    ) -> Box<TreeNode<TimelineClip>> {
        thread_local! {
            static AC: Arc<AudioClip> = Arc::new(AudioClip::import(&test_file_path("48000 16-bit.wav")).unwrap());
        }

        AC.with(|ac| {
            Box::new(TreeNode::new(TimelineClip::new(
                Timestamp::from_beat_units(start_beat_units),
                length_beat_units.map(|l| Timestamp::from_beat_units(l)),
                AudioClipReader::new(Arc::clone(&ac), max_buffer_size, 48_000),
            )))
        })
    }

    #[test]
    fn insert() {
        let mut t = TimelineTrack::new(0, Arc::new(AtomicU64::new(0)), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(3, Some(1), 100);
        let c2 = clip(1, Some(2), 100);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);
        }}

        // c2 should be inserted before c1
        t.relevant_clip.with_cursor_mut(|cur| {
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
        let mut t = TimelineTrack::new(0, Arc::clone(&pos), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(1, Some(1), 100);
        let c2 = clip(3, Some(2), 100);

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
            t.relevant_clip.with_cursor_mut(|cur| {
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
            t.relevant_clip.with_cursor_mut(|cur| {
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
            t.relevant_clip.with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
            let mut out = [0.0; BUFFER_SIZE * CHANNELS];
            t.output(&info, &mut out[..]);
            for &mut s in out.iter_mut() {
                assert_eq!(s, 0.0);
            }
            t.relevant_clip.with_cursor_mut(|cur| {
                let clip = cur.get();
                assert!(clip.is_none());
            });
        }}
    }

    #[test]
    fn output_many() {
        let mut t = TimelineTrack::new(0, Arc::new(AtomicU64::new(0)), SAMPLE_RATE, BPM_CENTS);
        let c1 = clip(0, Some(1), 200);
        let c2 = clip(2, Some(1), 200);
        let c3 = clip(4, Some(1), 200);
        let c4 = clip(5, Some(1), 200);

        no_heap! {{
            t.insert_clip(c1);
            t.insert_clip(c2);
            t.insert_clip(c3);
            t.insert_clip(c4);

            let mut out = [0.0; 6 * SBU * CHANNELS];
            t.output(&Info {
                sample_rate: SAMPLE_RATE,
                buffer_size: 6 * SBU,
            }, &mut out[..]);
            const SBUC: usize = SBU * CHANNELS;
            for &s in &out[..SBUC] {
                // Something
                assert_ne!(s, 0.0);
            }
            for &s in &out[SBUC..2 * SBUC] {
                // Nothing
                assert_eq!(s, 0.0);
            }
            for &s in &out[2 * SBUC..3 * SBUC] {
                // Something
                assert_ne!(s, 0.0);
            }
            for &s in &out[3 * SBUC..4 * SBUC] {
                // Nothing
                assert_eq!(s, 0.0);
            }
            for &s in &out[4 * SBUC..] {
                // Something
                assert_ne!(s, 0.0);
            }
        }}
    }
}
