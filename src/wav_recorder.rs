// Should be available for testing and debugging without necessarily being used elsewhere
#![allow(dead_code)]

use std::{fs, io};

pub struct WavRecorder {
    writer: hound::WavWriter<io::BufWriter<fs::File>>,
}

impl WavRecorder {
    pub fn new(channels: u16, sample_rate: u32) -> Self {
        const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/recorded.wav");

        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        WavRecorder {
            writer: hound::WavWriter::create(PATH, spec).unwrap(),
        }
    }

    pub fn record(&mut self, buffer: &[f32]) {
        for sample in buffer {
            self.writer.write_sample(*sample).unwrap();
        }
    }
}
