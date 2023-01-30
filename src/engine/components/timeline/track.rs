use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use intrusive_collections::rbtree::{CursorMut, RBTreeOps};
use intrusive_collections::{Adapter, RBTree};
use ouroboros::self_referencing;

use super::timeline_clip::TimelineClipAdapter;
use super::TimelineClip;
use crate::engine::components::track::TrackKey;
use crate::engine::traits::{Info, Source};
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

    clips: CursorOwning<TimelineClipAdapter>,

    output_track: TrackKey,

    output_buffer: Vec<Sample>,
}
impl TimelineTrack {
    pub fn new(
        output: TrackKey,
        position: Arc<AtomicU64>,
        sample_rate: u32,
        max_buffer_size: usize,
    ) -> Self {
        TimelineTrack {
            position,
            sample_rate,

            clips: CursorOwning::new(RBTree::new(TimelineClipAdapter::new()), |tree| {
                tree.front_mut()
            }),

            output_track: output,

            output_buffer: Vec::with_capacity(max_buffer_size * CHANNELS),
        }
    }

    pub fn output_track(&self) -> TrackKey {
        self.output_track
    }

    pub fn insert_clip(&mut self, clip: Box<TimelineClip>, bpm_cents: u16) {
        self.clips.with_cursor_mut(|cursor| {
            let position = Timestamp::from_samples(
                self.position.load(Ordering::SeqCst),
                self.sample_rate,
                bpm_cents,
            );
            let clip_end = clip.end(self.sample_rate, bpm_cents);
            let next_start = cursor.get().map_or(Timestamp::zero(), |next| next.start);

            let is_more_relevant = position < clip_end && clip_end < next_start;

            cursor.insert(clip);
            if is_more_relevant {
                cursor.move_prev();
            }
        })
    }
}
impl Debug for TimelineTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimelineTrack")
    }
}
impl Source for TimelineTrack {
    fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        todo!("output");

        &mut self.output_buffer[..buffer_size]
    }
}
