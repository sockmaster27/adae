use core::sync::atomic::Ordering;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BuildStreamError, Device, SampleFormat, Stream, StreamConfig};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

mod components;
mod utils;
pub use components::{Track, TrackData};

mod processor;
use self::components::mixer::{InvalidTrackError, TrackOverflowError, TrackReconstructionError};
use self::processor::{new_processor, Processor, ProcessorInterface};

/// Internally used sample format.
type Sample = f32;
/// Internally used channel count.
const CHANNELS: usize = 2;
/// Biggest possible requested buffer size.
const MAX_BUFFER_SIZE_DEFAULT: usize = 1056;

// CHANNELS and MAX_BUFFER_SIZE_DEFAULT are both usize, because they are mostly used for initializing and indexing Vec's.

pub struct Engine {
    /// Signal whether the stream should stop.
    stopped: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,

    processor_interface: ProcessorInterface,
}
impl Engine {
    pub fn new() -> Self {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("No ouput device available.");
        let supported_config = device.default_output_config().unwrap();
        let sample_format = supported_config.sample_format();
        let config = StreamConfig::from(supported_config);

        // Since buffer sizes can vary from output to output,
        // `max_buffer_size` denotes how much space each intermediate buffer should be initialized with.
        let max_buffer_size = match config.buffer_size {
            cpal::BufferSize::Fixed(size) => size as usize,
            cpal::BufferSize::Default => MAX_BUFFER_SIZE_DEFAULT,
        };
        let (processor_interface, processor) = new_processor(&config, max_buffer_size);

        let create_stream = match sample_format {
            SampleFormat::F32 => Self::create_stream::<f32>,
            SampleFormat::I16 => Self::create_stream::<i16>,
            SampleFormat::U16 => Self::create_stream::<u16>,
        };

        let stopped = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped);
        let join_handle = thread::spawn(move || {
            // Since cpal::Stream doesn't implement the Send trait, it has to live in this thread.

            let stream = create_stream(&device, &config, processor).unwrap();
            stream.play().unwrap();

            println!(
                "Host: {} \nDevice: {} \nSample size: {} bytes",
                host.id().name(),
                device.name().unwrap(),
                sample_format.sample_size()
            );

            while !stopped2.load(Ordering::Acquire) {
                // Parking the thread is more efficient than spinning, but can risk unparking seemingly randomly, hence the 'stopped' flag.
                thread::park();
            }

            // Just to be explicit
            drop(stream);
            println!("Stream terminated.");
        });

        let join_handle = Some(join_handle);

        Engine {
            stopped,
            join_handle,
            processor_interface,
        }
    }

    /// Create a cpal stream with the given sample type.
    fn create_stream<T: 'static + cpal::Sample>(
        device: &Device,
        config: &StreamConfig,
        mut processor: Processor,
    ) -> Result<Stream, BuildStreamError> {
        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _info| processor.output(data),
            |err| panic!("{}", err),
        )?;

        Ok(stream)
    }

    pub fn tracks(&self) -> Vec<&Track> {
        self.processor_interface.mixer.tracks()
    }
    pub fn tracks_mut(&mut self) -> Vec<&mut Track> {
        self.processor_interface.mixer.tracks_mut()
    }

    pub fn track(&self, key: u32) -> Result<&Track, InvalidTrackError> {
        self.processor_interface.mixer.track(key)
    }
    pub fn track_mut(&mut self, key: u32) -> Result<&mut Track, InvalidTrackError> {
        self.processor_interface.mixer.track_mut(key)
    }

    pub fn add_track(&mut self) -> Result<&mut Track, TrackOverflowError> {
        self.processor_interface.mixer.add_track()
    }
    pub fn reconstruct_track(
        &mut self,
        data: &TrackData,
    ) -> Result<&mut Track, TrackReconstructionError> {
        self.processor_interface.mixer.reconstruct_track(data)
    }
    pub fn delete_track(&mut self, key: u32) -> Result<(), InvalidTrackError> {
        self.processor_interface.mixer.delete_track(key)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let join_handle = self
            .join_handle
            .take()
            .expect("Stream was terminated more than once.");
        join_handle.thread().unpark();
        join_handle.join().unwrap();
    }
}
