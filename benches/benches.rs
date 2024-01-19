use std::path::Path;

use adae::{Engine, StoredAudioClipKey, Timestamp};

use criterion::{criterion_group, criterion_main, Criterion};

fn import_audio_clip(e: &mut Engine) -> StoredAudioClipKey {
    e.import_audio_clip(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test_files/44100 16-bit.wav"
    )))
    .unwrap()
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("Crop clip end", |b| {
        let (mut e, mut p) = Engine::dummy_with_processor();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(2)),
            )
            .unwrap();

        b.iter(|| {
            let _ = e.audio_clip_crop_end(ac, Timestamp::from_beats(1));
            p.poll();
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = criterion_benchmark
}
criterion_main!(benches);
