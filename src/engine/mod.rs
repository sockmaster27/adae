use core::sync::atomic::Ordering;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BuildStreamError, Device, SampleFormat, Stream, StreamConfig};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

mod components;

mod audio_thread;
use self::audio_thread::{new_audio_thread, AudioThread, AudioThreadInterface};

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

    audio_thread_interface: AudioThreadInterface,
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
        let (audio_thread_interface, audio_thread) = new_audio_thread(&config, max_buffer_size);

        let create_stream = match sample_format {
            SampleFormat::F32 => Self::create_stream::<f32>,
            SampleFormat::I16 => Self::create_stream::<i16>,
            SampleFormat::U16 => Self::create_stream::<u16>,
        };

        let stopped = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped);
        let join_handle = thread::spawn(move || {
            // Since cpal::Stream doesn't implement the Send trait, it has to live in this thread.

            let stream = create_stream(&device, &config, audio_thread).unwrap();
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
            audio_thread_interface,
        }
    }

    /// Create a cpal stream with the given sample type.
    fn create_stream<T: 'static + cpal::Sample>(
        device: &Device,
        config: &StreamConfig,
        mut audio_thread: AudioThread,
    ) -> Result<Stream, BuildStreamError> {
        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _info| audio_thread.output(data),
            |err| panic!("{}", err),
        )?;

        Ok(stream)
    }

    pub fn set_volume(&mut self, value: f32) {
        self.audio_thread_interface.set_gain(value);
    }

    /// Return an array of the signals current peak, long-term peak and RMS-level for each channel in the form:
    /// - `[peak: [left, right], long_peak: [left, right], rms: [left, right]]`
    pub fn get_meter(&self) -> [[Sample; CHANNELS]; 3] {
        self.audio_thread_interface.audio_meter.read()
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
