use core::sync::atomic::Ordering;
use cpal::traits::{DeviceTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::iter::zip;
use std::panic;
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

use crate::engine::utils::panic_msg;

pub use components::audio_clip_store::{ImportError, InvalidStoredAudioClipError};
pub use components::mixer::{InvalidMixerTrackError, MixerTrackOverflowError};
pub use components::stored_audio_clip::StoredAudioClip;
pub use components::stored_audio_clip::StoredAudioClipKey;
pub use components::timeline::AudioClip;
pub use components::timeline::AudioClipKey;
pub use components::timeline::AudioClipReconstructionError;
pub use components::timeline::AudioClipState;
pub use components::timeline::InvalidAudioClipError;
pub use components::timeline::InvalidAudioClipsError;
pub use components::timeline::MoveAudioClipToTrackError;
pub use components::timeline::Timestamp;
pub use components::timeline::{
    AddClipError, InvalidTimelineTrackError, MoveAudioClipError, TimelineTrackKey,
    TimelineTrackOverflowError, TimelineTrackState,
};
pub use components::MixerTrack;
pub use components::{MixerTrackKey, MixerTrackState};
use config::{Config, SampleFormat};
use config::{SampleFormatFloat, SampleFormatInt, SampleFormatIntUnsigned};
use processor::{processor, Processor, ProcessorInterface, ProcessorState};

use self::utils::key_generator::key_type;
use self::utils::key_generator::KeyGenerator;

/// Internally used sample format.
type Sample = f32;
/// Internally used channel count.
const CHANNELS: usize = 2;
/// Biggest possible requested buffer size.
const MAX_BUFFER_SIZE_DEFAULT: usize = 1056;
// CHANNELS and MAX_BUFFER_SIZE_DEFAULT are both usize, because they are mostly used for initializing and indexing Vec's.

key_type!(pub struct AudioTrackKey(u32));

struct StartedStream {
    stopped_flag: Arc<AtomicBool>,
    join_handle: JoinHandle<()>,
    processor_interface: ProcessorInterface,
    import_errors: Vec<ImportError>,
}

/// The Adae audio engine.
pub struct Engine {
    /// Signal whether the stream should stop.
    stopped: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,

    config: Config,
    processor_interface: ProcessorInterface,

    key_generator: KeyGenerator<AudioTrackKey>,
    audio_tracks: HashMap<AudioTrackKey, (TimelineTrackKey, MixerTrackKey)>,
}
impl Engine {
    /// Create a clean, empty instance of the engine with the default config.
    pub fn empty() -> Self {
        let (engine, import_errors) = Engine::new(Config::default(), &EngineState::default())
            .expect("Failed to create empty engine");
        debug_assert!(
            import_errors.count() == 0,
            "Empty engine should not have import errors"
        );

        engine
    }

    /// Create a new instance of the engine from the given state with the given config.
    pub fn new(
        config: Config,
        state: &EngineState,
    ) -> Result<(Self, impl Iterator<Item = ImportError>), InvalidConfigError> {
        let StartedStream {
            stopped_flag,
            join_handle,
            processor_interface,
            import_errors,
        } = Self::start_stream(&config, state)?;

        let engine = Engine {
            stopped: stopped_flag,
            join_handle: Some(join_handle),
            config,
            processor_interface,
            key_generator: KeyGenerator::from_iter(
                state.audio_tracks.iter().map(|(key, _, _)| *key),
            ),
            audio_tracks: HashMap::from_iter(state.audio_tracks.iter().map(
                |(key, timeline_track_key, mixer_track_key)| {
                    (*key, (*timeline_track_key, *mixer_track_key))
                },
            )),
        };

        Ok((engine, import_errors.into_iter()))
    }

    /// Starts a stream with the given config and state.
    ///
    /// Returns a the stop flag, the join handle, the processor interface and a (possibly empty) list of import errors.
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
                I8 => Self::create_stream_of_type::<i8>,
                I16 => Self::create_stream_of_type::<i16>,
                I32 => Self::create_stream_of_type::<i32>,
                I64 => Self::create_stream_of_type::<i64>,
            },
            IntUnsigned(s) => match s {
                U8 => Self::create_stream_of_type::<u8>,
                U16 => Self::create_stream_of_type::<u16>,
                U32 => Self::create_stream_of_type::<u32>,
                U64 => Self::create_stream_of_type::<u64>,
            },
            Float(s) => match s {
                F32 => Self::create_stream_of_type::<f32>,
                F64 => Self::create_stream_of_type::<f64>,
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

    /// Create a cpal stream with the given sample type.
    fn create_stream_of_type<T: 'static + cpal::SizedSample + cpal::FromSample<Sample>>(
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
                |err| panic!("{err}"),
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

    /// Creates an engine that simulates outputting without outputting to any audio device.
    ///
    /// Spins poll and output callback as fast as possible with a varying buffersize.  
    ///
    /// Useful for integration testing.
    #[doc(hidden)]
    pub fn dummy() -> Self {
        let (engine, import_errors) = Engine::dummy_from_state(&EngineState::default());
        debug_assert!(
            import_errors.count() == 0,
            "Empty engine should not have import errors"
        );

        engine
    }

    /// Like [`Engine::dummy()`], but uses the given state instead of the default state.
    #[doc(hidden)]
    pub fn dummy_from_state(state: &EngineState) -> (Self, impl Iterator<Item = ImportError>) {
        let (stopped, join_handle, processor_interface, import_errors) =
            Self::start_dummy_stream(state);

        let engine = Engine {
            stopped,
            join_handle: Some(join_handle),
            config: Config::dummy(),
            processor_interface,
            key_generator: KeyGenerator::from_iter(
                state.audio_tracks.iter().map(|(key, _, _)| *key),
            ),
            audio_tracks: HashMap::from_iter(state.audio_tracks.iter().map(
                |(key, timeline_track_key, mixer_track_key)| {
                    (*key, (*timeline_track_key, *mixer_track_key))
                },
            )),
        };

        (engine, import_errors.into_iter())
    }

    /// Creates an engine that simulates outputting without outputting to any audio device,
    /// while returning the processor to be poll and output manually.
    ///
    /// Useful for benchmarking.
    #[doc(hidden)]
    pub fn dummy_with_processor() -> (Self, Processor) {
        let (engine, processor, import_errors) =
            Engine::dummy_with_processor_from_state(&EngineState::default());
        debug_assert!(
            import_errors.count() == 0,
            "Empty engine should not have import errors"
        );

        (engine, processor)
    }

    /// Like [`Engine::dummy_with_processor()`], but uses the given state instead of the default state.
    #[doc(hidden)]
    pub fn dummy_with_processor_from_state(
        state: &EngineState,
    ) -> (Self, Processor, impl Iterator<Item = ImportError>) {
        let (processor_interface, processor, import_errors) = processor(
            &state.processor,
            &cpal::StreamConfig {
                channels: 2,
                sample_rate: cpal::SampleRate(48_000),
                buffer_size: cpal::BufferSize::Default,
            },
            1024,
        );

        let engine = Engine {
            stopped: Arc::new(AtomicBool::new(false)),
            join_handle: None,
            config: Config::dummy(),
            processor_interface,
            key_generator: KeyGenerator::from_iter(
                state.audio_tracks.iter().map(|(key, _, _)| *key),
            ),
            audio_tracks: HashMap::from_iter(state.audio_tracks.iter().map(
                |(key, timeline_track_key, mixer_track_key)| {
                    (*key, (*timeline_track_key, *mixer_track_key))
                },
            )),
        };

        (engine, processor, import_errors.into_iter())
    }

    /// Starts a stream that simulates outputting without outputting to any audio device.
    fn start_dummy_stream(
        state: &EngineState,
    ) -> (
        Arc<AtomicBool>,
        JoinHandle<()>,
        ProcessorInterface,
        impl Iterator<Item = ImportError>,
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

        (
            stopped1,
            join_handle,
            processor_interface,
            import_errors.into_iter(),
        )
    }

    /// Stops the stream if it is running.
    fn stop_stream(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(h) = self.join_handle.take() {
            h.thread().unpark();
            let r = h.join();
            if let Err(e) = r {
                let s = panic_msg(e);
                panic!("Failed to terminate stream: {s}");
            }
        }
    }

    /// Get the config that is currently in use.
    pub fn config(&self) -> &Config {
        &self.config
    }
    /// Restart the engine with the given config.
    pub fn set_config(&mut self, config: Config) -> Result<(), InvalidConfigError> {
        let state = self.state();

        self.stop_stream();

        let StartedStream {
            stopped_flag,
            join_handle,
            processor_interface,
            import_errors,
        } = Self::start_stream(&config, &state)?;

        debug_assert!(import_errors.is_empty());

        self.stopped = stopped_flag;
        self.join_handle = Some(join_handle);
        self.processor_interface = processor_interface;

        self.config = config;

        Ok(())
    }

    /// Get the current BPM multiplied by 100.
    ///
    /// For example, a return value of 12000 means 120 BPM.
    pub fn bpm_cents(&self) -> u16 {
        self.processor_interface.timeline.bpm_cents()
    }

    /// Play timeline from the current playhead position.
    pub fn play(&mut self) {
        self.processor_interface.timeline.play()
    }
    /// Pause playback of the timeline, without resetting the playhead position.
    pub fn pause(&mut self) {
        self.processor_interface.timeline.pause()
    }
    /// Set the current playhead position.
    ///
    /// This can be done both while the timeline is playing and while it is paused.
    pub fn jump_to(&mut self, position: Timestamp) {
        self.processor_interface.timeline.jump_to(position)
    }
    /// Get the current playhead position.
    ///
    /// This reports the position as it currently is on the audio thread, which might have a slight delay in reacting to [`Engine::jump_to()`].
    pub fn playhead_position(&mut self) -> Timestamp {
        self.processor_interface.timeline.playhead_position()
    }

    /// Immutably borrow the master track, which is always present on the mixer.
    pub fn master(&self) -> &MixerTrack {
        self.processor_interface.mixer.master()
    }
    /// Mutably borrow the master track, which is always present on the mixer.
    pub fn master_mut(&mut self) -> &mut MixerTrack {
        self.processor_interface.mixer.master_mut()
    }

    /// Get the keys of all audio tracks currently in the engine.
    pub fn audio_tracks(&self) -> impl Iterator<Item = AudioTrackKey> + '_ {
        self.audio_tracks.keys().copied()
    }

    /// Check if the engine contains an audio track with the given key.
    pub fn has_audio_track(&self, key: AudioTrackKey) -> bool {
        self.audio_tracks.contains_key(&key)
    }

    /// Get the key of the timeline track that corresponds to the given audio track.
    pub fn audio_timeline_track_key(
        &self,
        audio_track_key: AudioTrackKey,
    ) -> Result<TimelineTrackKey, InvalidAudioTrackError> {
        let &(timeline_track_key, _) =
            self.audio_tracks
                .get(&audio_track_key)
                .ok_or(InvalidAudioTrackError {
                    key: audio_track_key,
                })?;
        Ok(timeline_track_key)
    }

    /// Get the key of the mixer track that corresponds to the given audio track.
    pub fn audio_mixer_track_key(
        &self,
        audio_track_key: AudioTrackKey,
    ) -> Result<MixerTrackKey, InvalidAudioTrackError> {
        let &(_, mixer_track_key) =
            self.audio_tracks
                .get(&audio_track_key)
                .ok_or(InvalidAudioTrackError {
                    key: audio_track_key,
                })?;
        Ok(mixer_track_key)
    }

    /// Create new audio track, and add it to the engine.
    pub fn add_audio_track(&mut self) -> Result<AudioTrackKey, AudioTrackOverflowError> {
        if self.processor_interface.timeline.remaining_keys() == 0 {
            return Err(AudioTrackOverflowError::TimelineTracks(
                TimelineTrackOverflowError,
            ));
        }
        if self.processor_interface.mixer.remaining_keys() == 0 {
            return Err(AudioTrackOverflowError::MixerTracks(
                MixerTrackOverflowError,
            ));
        }
        if self.key_generator.remaining_keys() == 0 {
            return Err(AudioTrackOverflowError::AudioTracks);
        }

        let mixer_track_key = self.processor_interface.mixer.add_track().unwrap();
        let timeline_track_key = self
            .processor_interface
            .timeline
            .add_track(mixer_track_key)
            .unwrap();
        let audio_track_key = self.key_generator.next().unwrap();

        self.audio_tracks
            .insert(audio_track_key, (timeline_track_key, mixer_track_key));
        Ok(audio_track_key)
    }

    /// Create new set of tracks, and add them to the engine.
    pub fn add_audio_tracks(
        &mut self,
        count: u32,
    ) -> Result<impl Iterator<Item = AudioTrackKey>, AudioTrackOverflowError> {
        if self.processor_interface.timeline.remaining_keys() < count {
            return Err(AudioTrackOverflowError::TimelineTracks(
                TimelineTrackOverflowError,
            ));
        }
        if self.processor_interface.mixer.remaining_keys() < count {
            return Err(AudioTrackOverflowError::MixerTracks(
                MixerTrackOverflowError,
            ));
        }
        if self.key_generator.remaining_keys() < count {
            return Err(AudioTrackOverflowError::AudioTracks);
        }

        let mixer_track_keys = self.processor_interface.mixer.add_tracks(count).unwrap();
        let timeline_track_keys = self
            .processor_interface
            .timeline
            .add_tracks(mixer_track_keys.clone())
            .unwrap();
        let audio_track_keys: Vec<AudioTrackKey> =
            self.key_generator.next_n(count).unwrap().collect();

        for (&audio_track_key, tuple) in zip(
            &audio_track_keys,
            zip(timeline_track_keys, mixer_track_keys),
        ) {
            self.audio_tracks.insert(audio_track_key, tuple);
        }

        Ok(audio_track_keys.into_iter())
    }

    /// Delete audio track, and remove it from the engine.
    ///
    /// Returns a state that can be passed to [`Engine::reconstruct_audio_track()`]/[`Engine::reconstruct_audio_tracks()`],
    /// to reconstruct this track.
    pub fn delete_audio_track(
        &mut self,
        audio_track_key: AudioTrackKey,
    ) -> Result<AudioTrackState, InvalidAudioTrackError> {
        let &(timeline_track_key, mixer_track_key) = self
            .audio_tracks
            .get(&audio_track_key)
            .ok_or(InvalidAudioTrackError {
                key: audio_track_key,
            })?;

        let timeline_track_state = self
            .processor_interface
            .timeline
            .track_state(timeline_track_key)
            .unwrap();
        let mixer_track_state = self.mixer_track(mixer_track_key).unwrap().state();

        self.processor_interface
            .timeline
            .delete_track(timeline_track_key)
            .unwrap();
        self.processor_interface
            .mixer
            .delete_track(mixer_track_key)
            .unwrap();

        self.audio_tracks.remove(&audio_track_key);
        self.key_generator.free(audio_track_key).unwrap();

        Ok(AudioTrackState {
            key: audio_track_key,
            timeline_track_state,
            mixer_track_state,
        })
    }

    /// Delete a set of audio tracks, and remove them from the engine.
    ///
    /// Returns an iterator of states that can be passed to [`Engine::reconstruct_audio_track()`]/[`Engine::reconstruct_audio_tracks()`],
    /// to reconstruct these tracks.
    pub fn delete_audio_tracks(
        &mut self,
        audio_track_keys: impl IntoIterator<Item = AudioTrackKey>,
    ) -> Result<impl Iterator<Item = AudioTrackState>, InvalidAudioTracksError> {
        let audio_track_keys: Vec<AudioTrackKey> = audio_track_keys.into_iter().collect();

        let some_invalid = audio_track_keys
            .iter()
            .any(|&key| !self.key_generator.in_use(key));
        if some_invalid {
            let invalid_keys = audio_track_keys
                .iter()
                .filter(|&&key| !self.key_generator.in_use(key))
                .copied()
                .collect();
            return Err(InvalidAudioTracksError { keys: invalid_keys });
        }

        let timeline_track_keys: Vec<TimelineTrackKey> = audio_track_keys
            .iter()
            .map(|&key| self.audio_tracks.get(&key).unwrap().0)
            .collect();
        let mixer_track_keys: Vec<MixerTrackKey> = audio_track_keys
            .iter()
            .map(|&key| self.audio_tracks.get(&key).unwrap().1)
            .collect();

        let timeline_track_states = timeline_track_keys
            .iter()
            .map(|&key| self.processor_interface.timeline.track_state(key).unwrap());
        let mixer_track_states = mixer_track_keys
            .iter()
            .map(|&key| self.mixer_track(key).unwrap().state());

        let audio_track_states: Vec<AudioTrackState> = zip(
            audio_track_keys.iter(),
            zip(timeline_track_states, mixer_track_states),
        )
        .map(
            |(&key, (timeline_track_state, mixer_track_state))| AudioTrackState {
                key,
                timeline_track_state,
                mixer_track_state,
            },
        )
        .collect();

        self.processor_interface
            .timeline
            .delete_tracks(timeline_track_keys)
            .unwrap();
        self.processor_interface
            .mixer
            .delete_tracks(mixer_track_keys)
            .unwrap();

        for &key in audio_track_keys.iter() {
            self.audio_tracks.remove(&key);
            self.key_generator.free(key).unwrap();
        }

        Ok(audio_track_states.into_iter())
    }

    /// Reconstruct an audio track that has been deleted.
    ///
    /// A state can be obtained using [`Engine::audio_track_state()`].
    ///
    /// # Errors
    /// - [`AudioTrackReconstructionError::AudioTracks`] when the key of the audio track is already in use.
    /// - [`AudioTrackReconstructionError::TimelineTracks`] when the key of the timeline track is already in use.
    /// - [`AudioTrackReconstructionError::MixerTracks`] when the key of the mixer track is already in use.
    ///
    /// When a key is already in use, it means that the track either hasn't been deleted,
    /// or that the key has been repurposed for a new track.
    pub fn reconstruct_audio_track(
        &mut self,
        state: AudioTrackState,
    ) -> Result<AudioTrackKey, AudioTrackReconstructionError> {
        let audio_track_key = state.key;
        let timeline_track_key = state.timeline_track_state.key;
        let mixer_track_key = state.mixer_track_state.key;

        if self.key_generator.in_use(audio_track_key) {
            return Err(AudioTrackReconstructionError::AudioTracks(state.key));
        }
        if self
            .processor_interface
            .timeline
            .key_in_use(timeline_track_key)
        {
            return Err(AudioTrackReconstructionError::TimelineTracks(
                timeline_track_key,
            ));
        }
        if self.processor_interface.mixer.key_in_use(mixer_track_key) {
            return Err(AudioTrackReconstructionError::MixerTracks(mixer_track_key));
        }

        self.processor_interface
            .timeline
            .reconstruct_track(&state.timeline_track_state);
        self.processor_interface
            .mixer
            .reconstruct_track(&state.mixer_track_state);

        self.audio_tracks
            .insert(audio_track_key, (timeline_track_key, mixer_track_key));
        self.key_generator.reserve(audio_track_key).unwrap();

        Ok(audio_track_key)
    }

    /// Reconstruct a set of audio tracks that have been deleted.
    ///
    /// A state can be obtained using [`Engine::audio_track_state()`].
    ///
    /// # Errors
    /// - [`AudioTrackReconstructionError::AudioTracks`] when the key of the audio track is already in use.
    /// - [`AudioTrackReconstructionError::TimelineTracks`] when the key of the timeline track is already in use.
    /// - [`AudioTrackReconstructionError::MixerTracks`] when the key of the mixer track is already in use.
    ///
    /// When a key is already in use, it means that the track either hasn't been deleted,
    /// or that the key has been repurposed for a new track.
    pub fn reconstruct_audio_tracks<'a>(
        &mut self,
        states: impl IntoIterator<Item = AudioTrackState>,
    ) -> Result<impl Iterator<Item = AudioTrackKey> + 'a, AudioTrackReconstructionError> {
        let states_vec: Vec<AudioTrackState> = states.into_iter().collect();

        for state in &states_vec {
            let audio_track_key = state.key;
            let timeline_track_key = state.timeline_track_state.key;
            let mixer_track_key = state.mixer_track_state.key;

            if self.key_generator.in_use(audio_track_key) {
                return Err(AudioTrackReconstructionError::AudioTracks(state.key));
            }
            if self
                .processor_interface
                .timeline
                .key_in_use(timeline_track_key)
            {
                return Err(AudioTrackReconstructionError::TimelineTracks(
                    timeline_track_key,
                ));
            }
            if self.processor_interface.mixer.key_in_use(mixer_track_key) {
                return Err(AudioTrackReconstructionError::MixerTracks(mixer_track_key));
            }
        }

        self.processor_interface
            .timeline
            .reconstruct_tracks(states_vec.iter().map(|state| &state.timeline_track_state));
        self.processor_interface
            .mixer
            .reconstruct_tracks(states_vec.iter().map(|state| &state.mixer_track_state));

        for state in &states_vec {
            let audio_track_key = state.key;
            let timeline_track_key = state.timeline_track_state.key;
            let mixer_track_key = state.mixer_track_state.key;

            self.audio_tracks
                .insert(audio_track_key, (timeline_track_key, mixer_track_key));
            self.key_generator.reserve(audio_track_key).unwrap();
        }

        let audio_track_keys = states_vec.into_iter().map(|state| state.key);
        Ok(audio_track_keys)
    }

    /// Import audio clip from file.
    pub fn import_audio_clip(&mut self, path: &Path) -> Result<StoredAudioClipKey, ImportError> {
        self.processor_interface.timeline.import_audio_clip(path)
    }

    /// Get an imported audio clip.
    pub fn stored_audio_clip(
        &self,
        key: StoredAudioClipKey,
    ) -> Result<Arc<StoredAudioClip>, InvalidStoredAudioClipError> {
        self.processor_interface.timeline.stored_audio_clip(key)
    }

    /// Get all currently imported audio clips.
    pub fn stored_audio_clips(&self) -> impl Iterator<Item = Arc<StoredAudioClip>> + '_ {
        self.processor_interface.timeline.stored_audio_clips()
    }

    /// Add an audio clip to the given track's timeline.
    ///
    /// # Errors
    /// - [`AddClipError::InvalidTimelineTrack`] when the timeline track key is invalid.
    /// - [`AddClipError::InvalidClip`] when the stored audio clip key is invalid.
    /// - [`AddClipError::Overlapping`] when the clip would overlap with another clip on the same track.
    pub fn add_audio_clip(
        &mut self,
        timeline_track_key: TimelineTrackKey,
        clip_key: StoredAudioClipKey,
        start: Timestamp,
        length: Option<Timestamp>,
    ) -> Result<AudioClipKey, AddClipError> {
        self.processor_interface.timeline.add_audio_clip(
            timeline_track_key,
            clip_key,
            start,
            length,
        )
    }

    /// Get the audio clip with the given key.
    pub fn audio_clip(
        &self,
        audio_clip_key: AudioClipKey,
    ) -> Result<&AudioClip, InvalidAudioClipError> {
        self.processor_interface.timeline.audio_clip(audio_clip_key)
    }

    /// Get all audio clips on the given track.
    pub fn audio_clips(
        &self,
        timeline_track_key: TimelineTrackKey,
    ) -> Result<impl Iterator<Item = &AudioClip>, InvalidTimelineTrackError> {
        self.processor_interface
            .timeline
            .audio_clips(timeline_track_key)
    }

    /// Delete the audio clip with the given key.
    pub fn delete_audio_clip(
        &mut self,
        audio_clip_key: AudioClipKey,
    ) -> Result<(), InvalidAudioClipError> {
        self.processor_interface
            .timeline
            .delete_audio_clip(audio_clip_key)
    }

    /// Delete the audio clips with the given keys.
    pub fn delete_audio_clips(
        &mut self,
        audio_clip_keys: impl IntoIterator<Item = AudioClipKey>,
    ) -> Result<(), InvalidAudioClipsError> {
        self.processor_interface
            .timeline
            .delete_audio_clips(audio_clip_keys)
    }

    /// Reconstruct an audio clip that has been deleted.
    /// A state can be obtained using [`AudioClip::state()`].
    ///
    /// # Errors
    /// - [`AudioClipReconstructionError::InvalidTrack`] when the timeline track this clip was on no longer exists.
    /// - [`AudioClipReconstructionError::InvalidStoredClip`] when the stored audio clip this clip was based on no longer exists.
    /// - [`AudioClipReconstructionError::KeyInUse`] when the audio clip's key is already in use,
    ///     either because it was never deleted or because it has been repurposed for another clip.
    /// - [`AudioClipReconstructionError::Overlapping`] when the clip would overlap with another clip on the same track.
    pub fn reconstruct_audio_clip(
        &mut self,
        timneline_track_key: TimelineTrackKey,
        audio_clip_state: AudioClipState,
    ) -> Result<AudioClipKey, AudioClipReconstructionError> {
        self.processor_interface
            .timeline
            .reconstruct_audio_clip(timneline_track_key, audio_clip_state)
    }

    /// Reconstruct a set of audio clips that have been deleted.
    /// A state can be obtained using [`AudioClip::state()`].
    ///
    /// # Errors
    /// - [`AudioClipReconstructionError::InvalidTrack`] when the timeline track this clip was on no longer exists.
    /// - [`AudioClipReconstructionError::InvalidStoredClip`] when the stored audio clip this clip was based on no longer exists.
    /// - [`AudioClipReconstructionError::KeyInUse`] when the audio clip's key is already in use,
    ///     either because it was never deleted or because it has been repurposed for another clip.
    /// - [`AudioClipReconstructionError::Overlapping`] when the clip would overlap with another clip on the same track.
    pub fn reconstruct_audio_clips(
        &mut self,
        timeline_track_key: TimelineTrackKey,
        audio_clip_states: impl IntoIterator<Item = AudioClipState>,
    ) -> Result<impl Iterator<Item = AudioClipKey>, AudioClipReconstructionError> {
        self.processor_interface
            .timeline
            .reconstruct_audio_clips(timeline_track_key, audio_clip_states)
    }

    /// Set the start position of the clip.
    pub fn audio_clip_move(
        &mut self,
        audio_clip_key: AudioClipKey,
        new_start: Timestamp,
    ) -> Result<(), MoveAudioClipError> {
        self.processor_interface
            .timeline
            .audio_clip_move(audio_clip_key, new_start)
    }

    /// Move clip to the given position on another track.
    pub fn audio_clip_move_to_track(
        &mut self,
        audio_clip_key: AudioClipKey,
        new_start: Timestamp,
        new_timeline_track_key: TimelineTrackKey,
    ) -> Result<(), MoveAudioClipToTrackError> {
        self.processor_interface.timeline.audio_clip_move_to_track(
            audio_clip_key,
            new_start,
            new_timeline_track_key,
        )
    }

    /// Set the length of the clip, keeping the end position fixed.
    ///
    /// If this would result in the clip being extended past the beginning of the stored clip, or the beginning of the timeline, it will be capped to this length.
    /// The resulting start and length can queried from [`AudioClip::start()`] and [`AudioClip::length()`] after this.
    pub fn audio_clip_crop_start(
        &mut self,
        audio_clip_key: AudioClipKey,
        new_length: Timestamp,
    ) -> Result<(), MoveAudioClipError> {
        self.processor_interface
            .timeline
            .audio_clip_crop_start(audio_clip_key, new_length)
    }

    /// Set the length of the clip, keeping the start position fixed.
    ///
    /// If this results in the clip being extended past the end of the stored clip, the clip will be extended with silence.
    pub fn audio_clip_crop_end(
        &mut self,
        audio_clip_key: AudioClipKey,
        new_length: Timestamp,
    ) -> Result<(), MoveAudioClipError> {
        self.processor_interface
            .timeline
            .audio_clip_crop_end(audio_clip_key, new_length)
    }

    /// Get an immutable reference to the mixer track with the given key.
    pub fn mixer_track(&self, key: MixerTrackKey) -> Result<&MixerTrack, InvalidMixerTrackError> {
        self.processor_interface.mixer.track(key)
    }

    /// Get a mutable reference to the mixer track with the given key.
    pub fn mixer_track_mut(
        &mut self,
        key: MixerTrackKey,
    ) -> Result<&mut MixerTrack, InvalidMixerTrackError> {
        self.processor_interface.mixer.track_mut(key)
    }

    /// Get the current state of the engine.
    ///
    /// This can be used to recreate this exact state at a later time using [`Engine::new()`].
    pub fn state(&self) -> EngineState {
        EngineState {
            processor: self.processor_interface.state(),

            audio_tracks: self
                .audio_tracks
                .iter()
                .map(
                    |(&audio_track_key, &(timeline_track_key, mixer_track_key))| {
                        (audio_track_key, timeline_track_key, mixer_track_key)
                    },
                )
                .collect(),
        }
    }
}
impl Drop for Engine {
    /// Closes down the engine gracefully.
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
    audio_tracks: Vec<(AudioTrackKey, TimelineTrackKey, MixerTrackKey)>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrackState {
    key: AudioTrackKey,
    timeline_track_state: TimelineTrackState,
    mixer_track_state: MixerTrackState,
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
impl Error for InvalidConfigError {}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTrackOverflowError {
    MixerTracks(MixerTrackOverflowError),
    TimelineTracks(TimelineTrackOverflowError),
    /// Should not be reachable since the number of audio tracks is the minimum of the two
    AudioTracks,
}
impl Display for AudioTrackOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MixerTracks(e) => Display::fmt(&e, f),
            Self::TimelineTracks(e) => Display::fmt(&e, f),
            Self::AudioTracks => write!(f, "The max number of audio tracks has been exceeded"),
        }
    }
}
impl Error for AudioTrackOverflowError {}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAudioTrackError {
    key: AudioTrackKey,
}
impl Display for InvalidAudioTrackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = self.key;
        write!(f, "No track with key, {key:?}, on mixer")
    }
}
impl Error for InvalidAudioTrackError {}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidAudioTracksError {
    keys: Vec<AudioTrackKey>,
}
impl Display for InvalidAudioTracksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys = &self.keys;
        write!(f, "No tracks with keys: {keys:?}")
    }
}
impl Error for InvalidAudioTracksError {}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioTrackReconstructionError {
    AudioTracks(AudioTrackKey),
    TimelineTracks(TimelineTrackKey),
    MixerTracks(MixerTrackKey),
}
impl Display for AudioTrackReconstructionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AudioTracks(k) => write!(f, "Audio track key already in use: {k:?}"),
            Self::TimelineTracks(k) => write!(f, "Timeline track key already in use: {k:?}"),
            Self::MixerTracks(k) => write!(f, "Track key already in use: {k:?}"),
        }
    }
}
impl Error for AudioTrackReconstructionError {}
