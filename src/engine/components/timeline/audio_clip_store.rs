use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::engine::utils::key_generator::{self, KeyGenerator};

use super::audio_clip::{self, AudioClip};

pub type ClipKey = u32;
pub struct AudioClipStore {
    paths: HashMap<PathBuf, ClipKey>,
    clips: HashMap<ClipKey, AudioClip>,

    key_generator: KeyGenerator<ClipKey>,
}
impl AudioClipStore {
    pub fn import(&mut self, path: &Path, max_buffer_size: usize) -> Result<ClipKey, ImportError> {
        if let Some(&key) = self.paths.get(path) {
            // Clip is already imported
            return Ok(key);
        }

        let key = self.key_generator.next_key()?;

        let clip = AudioClip::import(path, max_buffer_size)?;

        // Commit only if no errors occur
        self.clips.insert(key, clip);
        self.paths.insert(path.to_owned(), key);

        Ok(key)
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
