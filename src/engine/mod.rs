use core::sync::atomic::Ordering;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BuildStreamError, Device, SampleFormat, SampleRate, Stream, StreamConfig};
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Display;
use std::iter::zip;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

mod components;
mod processor;
mod traits;
mod utils;

pub use self::components::audio_clip::AudioClip;
pub use self::components::audio_clip::AudioClipKey;
use self::components::audio_clip_store::{ImportError, InvalidAudioClipError};
use self::components::mixer::{InvalidMixerTrackError, MixerTrackOverflowError};
pub use self::components::timeline::Timestamp;
use self::components::timeline::{
    AddClipError, InvalidTimelineTrackError, TimelineTrackKey, TimelineTrackOverflowError,
    TimelineTrackState,
};
use self::components::{MixerTrackKey, MixerTrackState};
use self::processor::{processor, Processor, ProcessorInterface};
pub use components::MixerTrack;

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
    audio_tracks: HashSet<AudioTrack>,
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
        // `max_buffer_size` denotes how much space each intermediate buffer should be initialized with (per channel).
        let max_buffer_size = match config.buffer_size {
            // If usize is smaller than our buffersize we have bigger problems
            cpal::BufferSize::Fixed(size) => size.try_into().expect("Buffer size overflows usize"),
            cpal::BufferSize::Default => MAX_BUFFER_SIZE_DEFAULT,
        };
        let (processor_interface, processor) = processor(&config, max_buffer_size);

        use SampleFormat::*;
        let create_stream = match sample_format {
            I8 => Self::create_stream::<i8>,
            I16 => Self::create_stream::<i16>,
            I32 => Self::create_stream::<i32>,
            I64 => Self::create_stream::<i64>,

            U8 => Self::create_stream::<u8>,
            U16 => Self::create_stream::<u16>,
            U32 => Self::create_stream::<u32>,
            U64 => Self::create_stream::<u64>,

            F32 => Self::create_stream::<f32>,
            F64 => Self::create_stream::<f64>,

            _ => panic!("Unsupported sample format: {:?}", sample_format),
        };

        let stopped1 = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped1);
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
            stopped: stopped1,
            join_handle,
            processor_interface,
            audio_tracks: HashSet::new(),
        }
    }

    /// Create a cpal stream with the given sample type.
    fn create_stream<T: 'static + cpal::SizedSample + cpal::FromSample<Sample>>(
        device: &Device,
        config: &StreamConfig,
        mut processor: Processor,
    ) -> Result<Stream, BuildStreamError> {
        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _info| {
                no_heap! {{
                    processor.poll();
                    processor.output(data);
                }}
            },
            |err| panic!("{}", err),
            None,
        )?;

        Ok(stream)
    }

    /// Creates an engine that simulates outputting without outputting to any audio device.
    ///
    /// Spins poll and output callback as fast as possible with a varying buffersize.  
    ///
    /// Useful for integration testing.
    pub fn dummy() -> Self {
        let (processor_interface, mut processor) = processor(
            &StreamConfig {
                channels: 2,
                sample_rate: SampleRate(48_000),
                buffer_size: cpal::BufferSize::Default,
            },
            1024,
        );

        let mut data = vec![0.0; 2048];

        let stopped1 = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped1);
        let join_handle = thread::spawn(move || {
            while !stopped2.load(Ordering::Acquire) {
                let data = &mut data[..];
                no_heap! {{
                    processor.poll();
                    processor.output(data);
                }}
                let data = &mut data[..1024];
                no_heap! {{
                    processor.poll();
                    processor.output(data);
                }}
            }
        });

        Engine {
            stopped: stopped1,
            join_handle: Some(join_handle),
            processor_interface,
            audio_tracks: HashSet::new(),
        }
    }

    pub fn play(&mut self) {
        self.processor_interface.timeline.play()
    }
    pub fn pause(&mut self) {
        self.processor_interface.timeline.pause()
    }
    pub fn jump_to(&mut self, position: Timestamp) {
        self.processor_interface.timeline.jump_to(position)
    }
    pub fn playhead_position(&mut self) -> Timestamp {
        self.processor_interface.timeline.playhead_position()
    }

    pub fn import_audio_clip(&mut self, path: &Path) -> Result<AudioClipKey, ImportError> {
        self.processor_interface.timeline.import_audio_clip(path)
    }
    pub fn audio_clip(&self, key: AudioClipKey) -> Result<Arc<AudioClip>, InvalidAudioClipError> {
        self.processor_interface.timeline.audio_clip(key)
    }
    pub fn add_clip(
        &mut self,
        timeline_track_key: TimelineTrackKey,
        clip_key: AudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<(), AddClipError> {
        self.processor_interface
            .timeline
            .add_clip(timeline_track_key, clip_key, start, length)
    }

    pub fn master(&self) -> &MixerTrack {
        self.processor_interface.mixer.master()
    }
    pub fn master_mut(&mut self) -> &mut MixerTrack {
        self.processor_interface.mixer.master_mut()
    }

    pub fn mixer_track(&self, key: MixerTrackKey) -> Result<&MixerTrack, InvalidMixerTrackError> {
        self.processor_interface.mixer.track(key)
    }
    pub fn mixer_track_mut(
        &mut self,
        key: MixerTrackKey,
    ) -> Result<&mut MixerTrack, InvalidMixerTrackError> {
        self.processor_interface.mixer.track_mut(key)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &'_ AudioTrack> {
        self.audio_tracks.iter()
    }

    pub fn add_audio_track(&mut self) -> Result<AudioTrack, AudioTrackOverflowError> {
        if self.processor_interface.timeline.remaining_keys() == 0 {
            return Err(AudioTrackOverflowError::TimelineTracks(
                TimelineTrackOverflowError,
            ));
        }
        if self.processor_interface.mixer.remaining_keys() == 0 {
            return Err(AudioTrackOverflowError::Tracks(MixerTrackOverflowError));
        }

        let track_key = self.processor_interface.mixer.add_track().unwrap();
        let timeline_key = self
            .processor_interface
            .timeline
            .add_track(track_key)
            .unwrap();

        let audio_track = AudioTrack {
            timeline_track_key: timeline_key,
            track_key,
        };
        self.audio_tracks.insert(audio_track.clone());
        Ok(audio_track)
    }
    pub fn add_audio_tracks(
        &mut self,
        count: u32,
    ) -> Result<impl Iterator<Item = AudioTrack>, AudioTrackOverflowError> {
        if self.processor_interface.timeline.remaining_keys() < count {
            return Err(AudioTrackOverflowError::TimelineTracks(
                TimelineTrackOverflowError,
            ));
        }
        if self.processor_interface.mixer.remaining_keys() < count {
            return Err(AudioTrackOverflowError::Tracks(MixerTrackOverflowError));
        }

        let track_keys = self.processor_interface.mixer.add_tracks(count).unwrap();
        let timeline_keys = self
            .processor_interface
            .timeline
            .add_tracks(track_keys.clone())
            .unwrap();

        let audio_tracks =
            zip(track_keys, timeline_keys).map(|(track_key, timeline_key)| AudioTrack {
                timeline_track_key: timeline_key,
                track_key,
            });
        for audio_track in audio_tracks.clone() {
            self.audio_tracks.insert(audio_track.clone());
        }
        Ok(audio_tracks)
    }

    pub fn delete_audio_track(&mut self, track: AudioTrack) -> Result<(), InvalidAudioTrackError> {
        self.audio_track_exists(&track)?;

        self.audio_tracks.remove(&track);
        self.processor_interface
            .mixer
            .delete_track(track.track_key)
            .unwrap();
        self.processor_interface
            .timeline
            .delete_track(track.timeline_track_key)
            .unwrap();
        Ok(())
    }
    pub fn delete_audio_tracks(
        &mut self,
        tracks: Vec<AudioTrack>,
    ) -> Result<(), InvalidAudioTrackError> {
        let mut track_keys = Vec::with_capacity(tracks.len());
        let mut timeline_keys = Vec::with_capacity(tracks.len());
        for track in &tracks {
            self.audio_track_exists(&track)?;
        }
        for track in tracks {
            self.audio_tracks.remove(&track);
            track_keys.push(track.track_key);
            timeline_keys.push(track.timeline_track_key);
        }

        self.processor_interface
            .mixer
            .delete_tracks(track_keys)
            .unwrap();
        self.processor_interface
            .timeline
            .delete_tracks(timeline_keys)
            .unwrap();
        Ok(())
    }

    pub fn audio_track_state(
        &self,
        audio_track: &AudioTrack,
    ) -> Result<AudioTrackState, InvalidAudioTrackError> {
        self.audio_track_exists(audio_track)?;

        let timeline_track_state = self
            .processor_interface
            .timeline
            .track_state(audio_track.timeline_track_key())
            .unwrap();
        let track_state = self.mixer_track(audio_track.track_key()).unwrap().state();

        Ok(AudioTrackState {
            timeline_track_state,
            track_state,
        })
    }

    fn audio_track_exists(&self, audio_track: &AudioTrack) -> Result<(), InvalidAudioTrackError> {
        if !self
            .processor_interface
            .timeline
            .key_in_use(audio_track.timeline_track_key)
        {
            return Err(InvalidAudioTrackError::TimelineTracks(
                InvalidTimelineTrackError {
                    key: audio_track.timeline_track_key,
                },
            ));
        }
        if !self
            .processor_interface
            .mixer
            .key_in_use(audio_track.track_key)
        {
            return Err(InvalidAudioTrackError::Tracks(InvalidMixerTrackError {
                key: audio_track.track_key,
            }));
        }
        Ok(())
    }

    pub fn reconstruct_audio_track(
        &mut self,
        state: AudioTrackState,
    ) -> Result<AudioTrack, AudioTrackReconstructionError> {
        let timeline_track_key = state.timeline_track_state.key;
        let track_key = state.track_state.key;

        if self
            .processor_interface
            .timeline
            .key_in_use(timeline_track_key)
        {
            return Err(AudioTrackReconstructionError::TimelineTracks(
                timeline_track_key,
            ));
        }
        if self.processor_interface.mixer.key_in_use(track_key) {
            return Err(AudioTrackReconstructionError::Tracks(track_key));
        }

        self.processor_interface
            .timeline
            .reconstruct_track(&state.timeline_track_state, track_key);
        self.processor_interface
            .mixer
            .reconstruct_track(&state.track_state);

        let audio_track = AudioTrack {
            timeline_track_key,
            track_key,
        };

        self.audio_tracks.insert(audio_track.clone());

        Ok(audio_track)
    }
    pub fn reconstruct_audio_tracks(
        &mut self,
        states: Vec<AudioTrackState>,
    ) -> Result<Vec<AudioTrack>, AudioTrackReconstructionError> {
        for state in &states {
            let timeline_track_key = state.timeline_track_state.key;
            let track_key = state.track_state.key;

            if self
                .processor_interface
                .timeline
                .key_in_use(timeline_track_key)
            {
                return Err(AudioTrackReconstructionError::TimelineTracks(
                    timeline_track_key,
                ));
            }
            if self.processor_interface.mixer.key_in_use(track_key) {
                return Err(AudioTrackReconstructionError::Tracks(track_key));
            }
        }

        let mut audio_tracks = Vec::with_capacity(states.len());
        for state in &states {
            let timeline_track_key = state.timeline_track_state.key;
            let track_key = state.track_state.key;

            let audio_track = AudioTrack {
                timeline_track_key,
                track_key,
            };

            self.audio_tracks.insert(audio_track.clone());
            audio_tracks.push(audio_track);
        }

        self.processor_interface.timeline.reconstruct_tracks(
            states
                .iter()
                .map(|state| (&state.timeline_track_state, state.track_state.key)),
        );
        self.processor_interface
            .mixer
            .reconstruct_tracks(states.iter().map(|state| &state.track_state));

        Ok(audio_tracks)
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AudioTrack {
    timeline_track_key: TimelineTrackKey,
    track_key: MixerTrackKey,
}
impl AudioTrack {
    pub fn timeline_track_key(&self) -> TimelineTrackKey {
        self.timeline_track_key
    }
    pub fn track_key(&self) -> MixerTrackKey {
        self.track_key
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrackState {
    timeline_track_state: TimelineTrackState,
    track_state: MixerTrackState,
}

/// Scaling used by [`Track::read_meter`]
///
/// `∛|sample / 2|`
pub fn meter_scale(sample: Sample) -> Sample {
    (sample / 2.0).abs().powf(1.0 / 3.0)
}
/// Approximate inverse of scaling used by [`Track::read_meter`]
///
/// `2 ⋅ value³`
pub fn inverse_meter_scale(value: Sample) -> Sample {
    value.powi(3) * 2.0
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTrackOverflowError {
    Tracks(MixerTrackOverflowError),
    TimelineTracks(TimelineTrackOverflowError),
}
impl Display for AudioTrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tracks(e) => e.fmt(f),
            Self::TimelineTracks(e) => e.fmt(f),
        }
    }
}
impl Error for AudioTrackOverflowError {}

#[derive(Debug, PartialEq, Eq)]
pub enum InvalidAudioTrackError {
    Tracks(InvalidMixerTrackError),
    TimelineTracks(InvalidTimelineTrackError),
}
impl Display for InvalidAudioTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tracks(e) => e.fmt(f),
            Self::TimelineTracks(e) => e.fmt(f),
        }
    }
}
impl Error for InvalidAudioTrackError {}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTrackReconstructionError {
    Tracks(MixerTrackKey),
    TimelineTracks(TimelineTrackKey),
}
impl Display for AudioTrackReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tracks(k) => write!(f, "Track key already in use: {}", k),
            Self::TimelineTracks(k) => write!(f, "Timeline track key already in use: {}", k),
        }
    }
}
impl Error for AudioTrackReconstructionError {}
