use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    audio_clip_reader::AudioClipReader,
    stored_audio_clip::{self, StoredAudioClip, StoredAudioClipKey},
};
use crate::engine::utils::key_generator::{self, KeyGenerator};

pub struct AudioClipStore {
    max_buffer_size: usize,
    sample_rate: u32,

    paths: HashMap<PathBuf, StoredAudioClipKey>,
    clips: HashMap<StoredAudioClipKey, Arc<StoredAudioClip>>,

    key_generator: KeyGenerator<StoredAudioClipKey>,
}
impl AudioClipStore {
    /// Will reconstruct the store from the given state, importing all clips.
    ///
    /// Returns a list of errors that occured during import.
    /// If a track fails to import, it will be skipped.
    /// If the state contains no tracks, it is guaranteed that no errors will occur.
    ///
    /// # Panics
    /// If the state contains duplicate keys.
    pub fn new(
        state: &AudioClipStoreState,
        sample_rate: u32,
        max_buffer_size: usize,
    ) -> (Self, Vec<ImportError>) {
        let paths = HashMap::from_iter(
            state
                .clips
                .iter()
                .map(|(path, key)| (path.to_owned(), *key)),
        );

        let mut key_generator = KeyGenerator::new();

        let mut clips = HashMap::with_capacity(paths.len());
        let mut errors = Vec::new();
        for (path, &key) in paths.iter() {
            match StoredAudioClip::import(key, path) {
                Ok(clip) => {
                    key_generator
                        .reserve(key)
                        .expect("State contains duplicate keys");
                    clips.insert(key, Arc::new(clip));
                }
                Err(error) => errors.push(error.into()),
            }
        }

        let store = AudioClipStore {
            max_buffer_size,
            sample_rate,

            paths,
            clips,

            key_generator,
        };

        (store, errors)
    }

    pub fn import(&mut self, path: &Path) -> Result<StoredAudioClipKey, ImportError> {
        if let Some(&key) = self.paths.get(path) {
            // Clip is already imported
            return Ok(key);
        }

        let key = self.key_generator.next()?;

        let clip = StoredAudioClip::import(key, path)?;

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

    pub fn state(&self) -> AudioClipStoreState {
        AudioClipStoreState {
            clips: self
                .paths
                .iter()
                .map(|(path, &key)| (path.to_owned(), key))
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AudioClipStoreState {
    pub clips: Vec<(PathBuf, StoredAudioClipKey)>,
}
impl PartialEq for AudioClipStoreState {
    fn eq(&self, other: &Self) -> bool {
        let self_set: HashSet<_> = HashSet::from_iter(self.clips.iter());
        let other_set = HashSet::from_iter(other.clips.iter());

        debug_assert_eq!(
            self_set.len(),
            self.clips.len(),
            "Duplicate clips in AudioClipStoreState: {:?}",
            self.clips
        );
        debug_assert_eq!(
            other_set.len(),
            other.clips.len(),
            "Duplicate clips in AudioClipStoreState: {:?}",
            other.clips
        );

        self_set == other_set
    }
}
impl Eq for AudioClipStoreState {}

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
