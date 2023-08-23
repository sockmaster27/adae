use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::ops::RangeInclusive;

use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;

const PREFERRED_SAMPLE_RATE: u32 = 48_000;
const PREFERRED_BUFFER_SIZE: u32 = 512;

#[derive(Debug, Clone)]
pub struct Config {
    pub output_device: OutputDevice,
    pub output_config: OutputConfig,
}
impl Default for Config {
    fn default() -> Self {
        let host = Host::default();
        let output_device = host
            .default_output_device()
            .unwrap()
            .expect("No output device available");
        let output_config_range = output_device
            .default_config_range()
            .expect("No supported output config range available for default output device");
        let output_config = output_config_range.default_config();
        Self {
            output_device,
            output_config,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub channels: u16,
    pub sample_format: SampleFormat,
    pub sample_rate: u32,

    /// Buffer size in frames.
    /// If `None`, the default buffer size is used.
    pub buffer_size: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Host {
    name: String,
}
impl Host {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn available() -> impl Iterator<Item = Host> {
        cpal::available_hosts().into_iter().map(|id| Host {
            name: id.name().into(),
        })
    }

    pub fn output_devices(
        &self,
    ) -> Result<impl Iterator<Item = OutputDevice> + '_, HostUnavailableError> {
        Ok(self
            .raw()?
            .output_devices()
            .unwrap()
            .map(|device| OutputDevice {
                host: self.clone(),
                name: device.name().unwrap(),
            }))
    }

    pub fn default_output_device(&self) -> Result<Option<OutputDevice>, HostUnavailableError> {
        Ok(self
            .raw()?
            .default_output_device()
            .map(|device| OutputDevice {
                host: self.clone(),
                name: device.name().unwrap(),
            }))
    }

    pub(crate) fn raw(&self) -> Result<cpal::Host, HostUnavailableError> {
        let id = cpal::available_hosts()
            .into_iter()
            .find(|host| host.name() == self.name)
            .ok_or_else(|| HostUnavailableError {
                name: self.name.clone(),
            })?;

        match cpal::host_from_id(id) {
            Ok(host) => Ok(host),
            Err(cpal::HostUnavailable) => Err(HostUnavailableError {
                name: self.name.clone(),
            }),
        }
    }
}
impl Default for Host {
    fn default() -> Self {
        Self {
            name: cpal::default_host().id().name().into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutputDevice {
    host: Host,
    name: String,
}
impl OutputDevice {
    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn supported_config_ranges(
        &self,
    ) -> Result<impl Iterator<Item = OutputConfigRange>, DeviceUnavailableError> {
        Ok(self
            .raw()?
            .supported_output_configs()
            .unwrap()
            .map(|config| {
                let channels = config.channels();
                let sample_format = config.sample_format().into();
                let sample_rate = config.min_sample_rate().0..=config.max_sample_rate().0;
                let buffer_size = match config.buffer_size() {
                    cpal::SupportedBufferSize::Range { min, max } => Some((*min)..=(*max)),
                    cpal::SupportedBufferSize::Unknown => None,
                };
                OutputConfigRange {
                    channels,
                    sample_format,
                    sample_rate,
                    buffer_size,
                }
            }))
    }

    pub fn default_config_range(&self) -> Result<OutputConfigRange, DeviceUnavailableError> {
        let config = self.raw()?.default_output_config().unwrap();
        let channels = config.channels();
        let sample_format = config.sample_format().into();
        let sample_rate = config.sample_rate().0..=config.sample_rate().0;
        let buffer_size = match config.buffer_size() {
            cpal::SupportedBufferSize::Unknown => None,

            // This seems to be a bug in cpal (see https://github.com/RustAudio/cpal/issues/795)
            cpal::SupportedBufferSize::Range {
                min: u32::MIN,
                max: u32::MAX,
            } => None,

            cpal::SupportedBufferSize::Range { min, max } => Some((*min)..=(*max)),
        };
        Ok(OutputConfigRange {
            channels,
            sample_format,
            sample_rate,
            buffer_size,
        })
    }

    pub(crate) fn raw(&self) -> Result<cpal::Device, DeviceUnavailableError> {
        let host = self
            .host
            .raw()
            .map_err(|e| DeviceUnavailableError::HostUnavailable(e.name))?;

        host.output_devices()
            .unwrap()
            .find(|device| device.name().map(|name| name == self.name).unwrap_or(false))
            .ok_or_else(|| DeviceUnavailableError::DeviceUnavailable(self.name.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct OutputConfigRange {
    channels: u16,
    sample_format: SampleFormat,
    sample_rate: RangeInclusive<u32>,
    buffer_size: Option<RangeInclusive<u32>>,
}
impl OutputConfigRange {
    pub fn channels(&self) -> u16 {
        self.channels
    }
    pub fn sample_format(&self) -> &SampleFormat {
        &self.sample_format
    }
    pub fn sample_rate(&self) -> &RangeInclusive<u32> {
        &self.sample_rate
    }
    pub fn buffer_size(&self) -> Option<&RangeInclusive<u32>> {
        self.buffer_size.as_ref()
    }

    pub fn default_config(&self) -> OutputConfig {
        let sample_rate =
            PREFERRED_SAMPLE_RATE.clamp(*self.sample_rate.start(), *self.sample_rate.end());

        let buffer_size = self
            .buffer_size
            .as_ref()
            .map(|range| PREFERRED_BUFFER_SIZE.clamp(*range.start(), *range.end()));

        OutputConfig {
            channels: self.channels,
            sample_format: self.sample_format.clone(),
            sample_rate,
            buffer_size,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SampleFormat {
    Int(SampleFormatInt),
    IntUnsigned(SampleFormatIntUnsigned),
    Float(SampleFormatFloat),
}
#[derive(Debug, Clone)]
pub enum SampleFormatInt {
    I8,
    I16,
    I32,
    I64,
}
#[derive(Debug, Clone)]
pub enum SampleFormatIntUnsigned {
    U8,
    U16,
    U32,
    U64,
}
#[derive(Debug, Clone)]
pub enum SampleFormatFloat {
    F32,
    F64,
}
impl Display for SampleFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SampleFormat::Int(sample_format) => match sample_format {
                SampleFormatInt::I8 => write!(f, "8-bit"),
                SampleFormatInt::I16 => write!(f, "16-bit"),
                SampleFormatInt::I32 => write!(f, "32-bit"),
                SampleFormatInt::I64 => write!(f, "64-bit"),
            },
            SampleFormat::IntUnsigned(sample_format) => match sample_format {
                SampleFormatIntUnsigned::U8 => write!(f, "8-bit unsigned"),
                SampleFormatIntUnsigned::U16 => write!(f, "16-bit unsigned"),
                SampleFormatIntUnsigned::U32 => write!(f, "32-bit unsigned"),
                SampleFormatIntUnsigned::U64 => write!(f, "64-bit unsigned"),
            },
            SampleFormat::Float(sample_format) => match sample_format {
                SampleFormatFloat::F32 => write!(f, "32-bit floating point"),
                SampleFormatFloat::F64 => write!(f, "64-bit floating point"),
            },
        }
    }
}
impl From<cpal::SampleFormat> for SampleFormat {
    fn from(sample_format: cpal::SampleFormat) -> Self {
        match sample_format {
            cpal::SampleFormat::I8 => Self::Int(SampleFormatInt::I8),
            cpal::SampleFormat::I16 => Self::Int(SampleFormatInt::I16),
            cpal::SampleFormat::I32 => Self::Int(SampleFormatInt::I32),
            cpal::SampleFormat::I64 => Self::Int(SampleFormatInt::I64),
            cpal::SampleFormat::U8 => Self::IntUnsigned(SampleFormatIntUnsigned::U8),
            cpal::SampleFormat::U16 => Self::IntUnsigned(SampleFormatIntUnsigned::U16),
            cpal::SampleFormat::U32 => Self::IntUnsigned(SampleFormatIntUnsigned::U32),
            cpal::SampleFormat::U64 => Self::IntUnsigned(SampleFormatIntUnsigned::U64),
            cpal::SampleFormat::F32 => Self::Float(SampleFormatFloat::F32),
            cpal::SampleFormat::F64 => Self::Float(SampleFormatFloat::F64),
            _ => panic!("Unsupported sample format"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostUnavailableError {
    name: String,
}
impl Display for HostUnavailableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Host '{}' is not available", self.name)
    }
}
impl Error for HostUnavailableError {}

#[derive(Debug, Clone)]
pub enum DeviceUnavailableError {
    HostUnavailable(String),
    DeviceUnavailable(String),
}
impl Display for DeviceUnavailableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceUnavailableError::HostUnavailable(name) => {
                write!(f, "Host '{}' is not available", name)
            }
            DeviceUnavailableError::DeviceUnavailable(name) => {
                write!(f, "Device '{}' is not available", name)
            }
        }
    }
}
impl Error for DeviceUnavailableError {}
