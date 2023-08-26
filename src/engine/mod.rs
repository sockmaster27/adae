use core::sync::atomic::Ordering;
use cpal::traits::{DeviceTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::iter::zip;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

mod components;
pub mod config;
pub mod error;
mod info;
mod processor;
mod utils;

pub use components::audio_clip_store::{ImportError, InvalidAudioClipError};
pub use components::mixer::{InvalidMixerTrackError, MixerTrackOverflowError};
pub use components::stored_audio_clip::StoredAudioClip;
pub use components::stored_audio_clip::StoredAudioClipKey;
pub use components::timeline::Timestamp;
pub use components::timeline::{
    AddClipError, InvalidTimelineTrackError, TimelineTrackKey, TimelineTrackOverflowError,
    TimelineTrackState,
};
pub use components::MixerTrack;
pub use components::{MixerTrackKey, MixerTrackState};
use config::{Config, SampleFormat};
use config::{SampleFormatFloat, SampleFormatInt, SampleFormatIntUnsigned};
use processor::{processor, Processor, ProcessorInterface, ProcessorState};

/// Internally used sample format.
type Sample = f32;
/// Internally used channel count.
const CHANNELS: usize = 2;
/// Biggest possible requested buffer size.
const MAX_BUFFER_SIZE_DEFAULT: usize = 1056;

// CHANNELS and MAX_BUFFER_SIZE_DEFAULT are both usize, because they are mostly used for initializing and indexing Vec's.

struct StartedStream {
    stopped_flag: Arc<AtomicBool>,
    join_handle: JoinHandle<()>,
    processor_interface: ProcessorInterface,
    import_errors: Vec<ImportError>,
}

pub struct Engine {
    /// Signal whether the stream should stop.
    stopped: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,

    processor_interface: ProcessorInterface,
    audio_tracks: HashSet<AudioTrack>,
}
impl Engine {
    pub fn empty() -> Self {
        let (engine, import_errors) = Engine::new(&Config::default(), &EngineState::default())
            .expect("Failed to create empty engine");
        debug_assert!(
            import_errors.is_empty(),
            "Empty engine should not have import errors"
        );

        engine
    }

    pub fn new(
        config: &Config,
        state: &EngineState,
    ) -> Result<(Self, Vec<ImportError>), InvalidConfigError> {
        let StartedStream {
            stopped_flag,
            join_handle,
            processor_interface,
            import_errors,
        } = Self::start_stream(config, state)?;

        let engine = Engine {
            stopped: stopped_flag,
            join_handle: Some(join_handle),
            processor_interface,
            audio_tracks: HashSet::from_iter(state.audio_tracks.iter().cloned()),
        };

        Ok((engine, import_errors))
    }

    /// Restart the engine with the given config.
    pub fn set_config(&mut self, config: &Config) -> Result<(), InvalidConfigError> {
        let state = self.state();

        self.stop_stream();

        let StartedStream {
            stopped_flag,
            join_handle,
            processor_interface,
            import_errors,
        } = Self::start_stream(config, &state)?;

        debug_assert!(import_errors.is_empty());

        self.stopped = stopped_flag;
        self.join_handle = Some(join_handle);
        self.processor_interface = processor_interface;

        Ok(())
    }

    /// Creates an engine that simulates outputting without outputting to any audio device.
    ///
    /// Spins poll and output callback as fast as possible with a varying buffersize.  
    ///
    /// Useful for integration testing.
    pub fn dummy() -> Self {
        let (engine, import_errors) = Engine::dummy_from_state(&EngineState::default());
        debug_assert!(
            import_errors.is_empty(),
            "Empty engine should not have import errors"
        );

        engine
    }

    /// Like [`Engine::dummy()`], but uses the given state instead of the default state.
    pub fn dummy_from_state(state: &EngineState) -> (Self, Vec<ImportError>) {
        let (stopped, join_handle, processor_interface, import_errors) =
            Self::start_dummy_stream(state);

        let engine = Engine {
            stopped,
            join_handle: Some(join_handle),
            processor_interface,
            audio_tracks: HashSet::from_iter(state.audio_tracks.iter().cloned()),
        };

        (engine, import_errors)
    }

    /// Starts a stream with the given config and state.
    ///
    /// Returns a the stop flag, the join handle, the processor interface and a list of import errors.
    fn start_stream(
        config: &Config,
        state: &EngineState,
    ) -> Result<StartedStream, InvalidConfigError> {
        let device = config.output_device.clone();
        let output_config = config.output_config.clone();
        let stream_config = cpal::StreamConfig {
            channels: output_config.channels,
            sample_rate: cpal::SampleRate(output_config.sample_rate),
            buffer_size: match output_config.buffer_size {
                Some(size) => cpal::BufferSize::Fixed(size),
                None => cpal::BufferSize::Default,
            },
        };

        // Since buffer sizes can vary from output to output,
        // `max_buffer_size` denotes how much space each intermediate buffer should be initialized with (per channel).
        let max_buffer_size = match output_config.buffer_size {
            // If usize is smaller than our buffersize we have bigger problems
            Some(size) => size.try_into().expect("Buffer size overflows usize"),
            None => MAX_BUFFER_SIZE_DEFAULT,
        };
        let (processor_interface, processor, import_errors) =
            processor(&state.processor, &stream_config, max_buffer_size);

        use SampleFormat::*;
        use SampleFormatFloat::*;
        use SampleFormatInt::*;
        use SampleFormatIntUnsigned::*;
        let create_stream = match output_config.sample_format.clone() {
            Int(s) => match s {
                I8 => Self::create_stream::<i8>,
                I16 => Self::create_stream::<i16>,
                I32 => Self::create_stream::<i32>,
                I64 => Self::create_stream::<i64>,
            },
            IntUnsigned(s) => match s {
                U8 => Self::create_stream::<u8>,
                U16 => Self::create_stream::<u16>,
                U32 => Self::create_stream::<u32>,
                U64 => Self::create_stream::<u64>,
            },
            Float(s) => match s {
                F32 => Self::create_stream::<f32>,
                F64 => Self::create_stream::<f64>,
            },
        };

        let (tx, rx) = sync_channel(1);

        let stopped1 = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped1);
        let join_handle = thread::spawn(move || {
            // Since cpal::Stream doesn't implement the Send trait, it has to live in this thread.

            let res = create_stream(&device.raw().unwrap(), &stream_config, processor);

            let stream = match res {
                Ok(stream) => {
                    tx.send(None).unwrap();
                    stream
                }
                Err(e) => {
                    tx.send(Some(e)).unwrap();
                    return;
                }
            };

            stream.play().unwrap();

            println!(
                "Host: {}\nDevice: {}\nChannels: {}\nSample format: {}\nSample rate: {}\nBuffer size: {}",
                device.host().name(),
                device.name(),
                output_config.channels,
                output_config.sample_format,
                output_config.sample_rate,
                output_config.buffer_size.map(|s| s.to_string()).unwrap_or("Default".into()),

            );

            while !stopped2.load(Ordering::Acquire) {
                // Parking the thread is more efficient than spinning, but can risk unparking seemingly randomly, hence the 'stopped' flag.
                thread::park();
            }

            // Just to be explicit
            drop(stream);
            println!("Stream terminated");
        });

        let res = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("Attempt to start stream timed out");

        match res {
            Some(e) => Err(e),
            None => Ok(StartedStream {
                stopped_flag: stopped1,
                join_handle,
                processor_interface,
                import_errors,
            }),
        }
    }

    /// Starts a stream that simulates outputting without outputting to any audio device.
    fn start_dummy_stream(
        state: &EngineState,
    ) -> (
        Arc<AtomicBool>,
        JoinHandle<()>,
        ProcessorInterface,
        Vec<ImportError>,
    ) {
        let (processor_interface, mut processor, import_errors) = processor(
            &state.processor,
            &cpal::StreamConfig {
                channels: 2,
                sample_rate: cpal::SampleRate(48_000),
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

        (stopped1, join_handle, processor_interface, import_errors)
    }

    /// Stops the stream if it is running.
    fn stop_stream(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(h) = self.join_handle.take() {
            h.thread().unpark();
            let r = h.join();
            if let Err(e) = r {
                if let Ok(d) = e.downcast::<String>() {
                    panic!("Failed to terminate stream: {d:?}");
                } else {
                    panic!("Failed to terminate stream");
                }
            }
        }
    }

    /// Create a cpal stream with the given sample type.
    fn create_stream<T: 'static + cpal::SizedSample + cpal::FromSample<Sample>>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut processor: Processor,
    ) -> Result<cpal::Stream, InvalidConfigError> {
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _info| {
                    no_heap! {{
                        processor.poll();
                        processor.output(data);
                    }}
                },
                |err| panic!("{}", err),
                None,
            )
            .map_err(|e| match e {
                cpal::BuildStreamError::DeviceNotAvailable => {
                    InvalidConfigError::DeviceNotAvailable
                }
                cpal::BuildStreamError::StreamConfigNotSupported => InvalidConfigError::Other,
                cpal::BuildStreamError::InvalidArgument => InvalidConfigError::Other,

                e => panic!("Stream could not be created: {e}"),
            })
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

    pub fn import_audio_clip(&mut self, path: &Path) -> Result<StoredAudioClipKey, ImportError> {
        self.processor_interface.timeline.import_audio_clip(path)
    }
    pub fn stored_audio_clip(
        &self,
        key: StoredAudioClipKey,
    ) -> Result<Arc<StoredAudioClip>, InvalidAudioClipError> {
        self.processor_interface.timeline.stored_audio_clip(key)
    }
    pub fn add_clip(
        &mut self,
        timeline_track_key: TimelineTrackKey,
        clip_key: StoredAudioClipKey,
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
            self.audio_track_exists(track)?;
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

    pub fn state(&self) -> EngineState {
        EngineState {
            processor: self.processor_interface.state(),

            audio_tracks: self.audio_tracks.iter().cloned().collect(),
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_stream();
    }
}

/// The state of the [`Engine`].
///
/// This can be used to recreate this exact state at a later time,
/// suitable for saving to a file.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EngineState {
    processor: ProcessorState,

    // AudioTrackState is not used because the individual track's states are kept in the mixer and timeline's state.
    audio_tracks: Vec<AudioTrack>,
}
impl PartialEq for EngineState {
    fn eq(&self, other: &Self) -> bool {
        let self_set: HashSet<_> = HashSet::from_iter(self.audio_tracks.iter());
        let other_set = HashSet::from_iter(other.audio_tracks.iter());

        debug_assert_eq!(
            self_set.len(),
            self.audio_tracks.len(),
            "Duplicate audio tracks in EngineState: {:?}",
            self.audio_tracks
        );
        debug_assert_eq!(
            other_set.len(),
            other.audio_tracks.len(),
            "Duplicate audio tracks in EngineState: {:?}",
            other.audio_tracks
        );

        self.processor == other.processor && self_set == other_set
    }
}
impl Eq for EngineState {}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
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

/// Scaling used by [`MixerTrack::read_meter`]
///
/// `∛|sample / 2|`
pub fn meter_scale(sample: Sample) -> Sample {
    (sample / 2.0).abs().powf(1.0 / 3.0)
}
/// Approximate inverse of scaling used by [`MixerTrack::read_meter`]
///
/// `2 ⋅ value³`
pub fn inverse_meter_scale(value: Sample) -> Sample {
    value.powi(3) * 2.0
}

#[derive(Debug, PartialEq, Eq)]
pub enum InvalidConfigError {
    DeviceNotAvailable,
    Other,
}
impl Display for InvalidConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidConfigError::DeviceNotAvailable => write!(
                f,
                "Engine received unsupported conifguration: Device is not available"
            ),
            InvalidConfigError::Other => write!(f, "Engine received unsupported conifguration"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTrackOverflowError {
    Tracks(MixerTrackOverflowError),
    TimelineTracks(TimelineTrackOverflowError),
}
impl Display for AudioTrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tracks(e) => Display::fmt(&e, f),
            Self::TimelineTracks(e) => Display::fmt(&e, f),
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
            Self::Tracks(e) => Display::fmt(&e, f),
            Self::TimelineTracks(e) => Display::fmt(&e, f),
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
