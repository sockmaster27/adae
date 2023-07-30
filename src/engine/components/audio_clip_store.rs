use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::engine::utils::key_generator::{self, KeyGenerator};

use super::{
    audio_clip_reader::AudioClipReader,
    stored_audio_clip::{self, StoredAudioClip, StoredAudioClipKey},
};

pub struct AudioClipStore {
    max_buffer_size: usize,
    sample_rate: u32,

    paths: HashMap<PathBuf, StoredAudioClipKey>,
    clips: HashMap<StoredAudioClipKey, Arc<StoredAudioClip>>,

    key_generator: KeyGenerator<StoredAudioClipKey>,
}
impl AudioClipStore {
    pub fn new(max_buffer_size: usize, sample_rate: u32) -> Self {
        AudioClipStore {
            max_buffer_size,
            sample_rate,

            paths: HashMap::new(),
            clips: HashMap::new(),

            key_generator: KeyGenerator::new(),
        }
    }

    pub fn import(&mut self, path: &Path) -> Result<StoredAudioClipKey, ImportError> {
        if let Some(&key) = self.paths.get(path) {
            // Clip is already imported
            return Ok(key);
        }

        let key = self.key_generator.next()?;

        let clip = StoredAudioClip::import(path)?;

        // Commit only if no errors occur
        self.clips.insert(key, Arc::new(clip));
        self.paths.insert(path.to_owned(), key);

        Ok(key)
    }

    pub fn get(
        &self,
        key: StoredAudioClipKey,
    ) -> Result<Arc<StoredAudioClip>, InvalidAudioClipError> {
        match self.clips.get(&key) {
            None => Err(InvalidAudioClipError { key }),
            Some(clip) => Ok(Arc::clone(clip)),
        }
    }
    pub fn reader(
        &self,
        key: StoredAudioClipKey,
    ) -> Result<AudioClipReader, InvalidAudioClipError> {
        let clip = self.get(key)?;
        Ok(AudioClipReader::new(
            clip,
            self.max_buffer_size,
            self.sample_rate,
        ))
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
pub struct InvalidAudioClipError {
    pub key: StoredAudioClipKey,
}
impl Display for InvalidAudioClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "No audio clip with key, {}, in audio clip store",
            self.key
        )
    }
}
impl Error for InvalidAudioClipError {}

#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    OverFlow(ClipOverflowError),
    Other(stored_audio_clip::ImportError),
}
impl Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverFlow(e) => e.fmt(f),
            Self::Other(e) => e.fmt(f),
        }
    }
}
impl Error for ImportError {}
impl From<key_generator::OverflowError> for ImportError {
    fn from(_: key_generator::OverflowError) -> Self {
        ImportError::OverFlow(ClipOverflowError)
    }
}
impl From<stored_audio_clip::ImportError> for ImportError {
    fn from(e: stored_audio_clip::ImportError) -> Self {
        ImportError::Other(e)
    }
}
