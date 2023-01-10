use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::engine::{
    components::event_queue::{EventQueue, EventReceiver},
    traits::Component,
    utils::{
        key_generator::{self, KeyGenerator},
        remote_push::{RemotePushable, RemotePushedHashMap, RemotePusherHashMap},
    },
};

use super::audio_clip::{self, AudioClip, AudioClipKey};

pub fn audio_clip_store(
    event_queue: &mut EventQueue,
    max_buffer_size: usize,
) -> (AudioClipStore, AudioClipStoreProcessor) {
    let (clips_pusher, clips_pushed) = HashMap::remote_push(event_queue);

    (
        AudioClipStore {
            max_buffer_size,

            paths: HashMap::new(),
            clips: clips_pusher,

            key_generator: KeyGenerator::new(),
        },
        AudioClipStoreProcessor {
            clips: clips_pushed,
        },
    )
}

pub struct AudioClipStore {
    max_buffer_size: usize,

    paths: HashMap<PathBuf, AudioClipKey>,
    clips: RemotePusherHashMap<AudioClipKey, AudioClip>,

    key_generator: KeyGenerator<AudioClipKey>,
}
impl AudioClipStore {
    pub fn import(&mut self, path: &Path) -> Result<AudioClipKey, ImportError> {
        if let Some(&key) = self.paths.get(path) {
            // Clip is already imported
            return Ok(key);
        }

        let key = self.key_generator.next_key()?;

        let clip = AudioClip::import(key, path, self.max_buffer_size)?;

        // Commit only if no errors occur
        self.clips.push((key, clip));
        self.paths.insert(path.to_owned(), key);

        Ok(key)
    }
}

#[derive(Debug)]
pub struct AudioClipStoreProcessor {
    clips: RemotePushedHashMap<AudioClipKey, AudioClip>,
}
impl Component for AudioClipStoreProcessor {
    fn poll<'a, 'b>(&'a mut self, event_receiver: &mut EventReceiver<'a, 'b>) {
        self.clips.poll(event_receiver);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClipOverflowError;
impl Display for ClipOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of audio clips has been exceeded.")
    }
}
impl Error for ClipOverflowError {}

#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    OverFlow(ClipOverflowError),
    Other(audio_clip::ImportError),
}
impl From<key_generator::OverflowError> for ImportError {
    fn from(_: key_generator::OverflowError) -> Self {
        ImportError::OverFlow(ClipOverflowError)
    }
}
impl From<audio_clip::ImportError> for ImportError {
    fn from(e: audio_clip::ImportError) -> Self {
        ImportError::Other(e)
    }
}
