use std::path::Path;

use adae::{Engine, StoredAudioClipKey};

pub fn import_audio_clip(e: &mut Engine) -> StoredAudioClipKey {
    e.import_audio_clip(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test_files/44100 16-bit.wav"
    )))
    .unwrap()
}
