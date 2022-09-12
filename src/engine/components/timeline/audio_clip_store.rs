use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    path::{Path, PathBuf},
};

use super::audio_clip::{self, AudioClip};

pub type ClipKey = u32;
pub struct AudioClipStore {
    paths: HashMap<PathBuf, ClipKey>,
    clips: HashMap<ClipKey, AudioClip>,

    last_key: ClipKey,
}
impl AudioClipStore {
    pub fn import(&mut self, path: &Path, max_buffer_size: usize) -> Result<ClipKey, ImportError> {
        if let Some(&key) = self.paths.get(path) {
            // Clip is already imported
            return Ok(key);
        }

        let key = self
            .next_key_after(self.last_key)
            .or_else(|e| Err(ImportError::OverFlow(e)))?;
        let clip =
            AudioClip::import(path, max_buffer_size).or_else(|e| Err(ImportError::Other(e)))?;

        // Commit only if no errors occur
        self.clips.insert(key, clip);
        self.paths.insert(path.to_owned(), key);
        self.last_key = key;

        Ok(key)
    }
    fn next_key_after(&self, last_key: ClipKey) -> Result<ClipKey, ClipOverflowError> {
        let mut key = last_key.wrapping_add(1);

        let mut i = 0;
        while self.clips.contains_key(&key) {
            i += 1;
            if i == ClipKey::MAX {
                return Err(ClipOverflowError);
            }

            key = key.wrapping_add(1);
        }

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

enum ImportError {
    OverFlow(ClipOverflowError),
    Other(audio_clip::ImportError),
}
