use std::{
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use ctru::services::ndsp::{self, wave, AudioFormat};

use crate::{
    host::ctru3ds::{ChannelConfig, WaveWrap},
    traits::DeviceTrait,
    Data, OutputCallbackInfo, OutputStreamTimestamp, SampleFormat, StreamInstant,
    SupportedBufferSize, SupportedStreamConfigRange,
};

pub type SupportedInputConfigs = std::iter::Once<SupportedStreamConfigRange>;
pub type SupportedOutputConfigs = std::iter::Once<SupportedStreamConfigRange>;

use super::HostData;

#[derive(Clone)]
pub struct Device {
    data: Pin<Arc<HostData>>,
}
pub struct Devices(pub Option<Device>);
impl Iterator for Devices {
    type Item = Device;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.take()
    }
}

impl Device {
    pub fn new(data: Pin<Arc<HostData>>) -> Self {
        Self { data }
    }
}
impl DeviceTrait for Device {
    type SupportedInputConfigs = SupportedInputConfigs;

    type SupportedOutputConfigs = SupportedOutputConfigs;

    type Stream = super::stream::Stream;

    fn name(&self) -> Result<String, crate::DeviceNameError> {
        Ok("ndsp".to_owned())
    }

    fn supported_input_configs(
        &self,
    ) -> Result<Self::SupportedInputConfigs, crate::SupportedStreamConfigsError> {
        todo!()
    }

    fn supported_output_configs(
        &self,
    ) -> Result<Self::SupportedOutputConfigs, crate::SupportedStreamConfigsError> {
        let def = self.default_output_config().unwrap();
        Ok(std::iter::once(SupportedStreamConfigRange {
            channels: def.channels,
            min_sample_rate: def.sample_rate,
            max_sample_rate: def.sample_rate,
            buffer_size: def.buffer_size,
            sample_format: def.sample_format,
        }))
    }

    fn default_input_config(
        &self,
    ) -> Result<crate::SupportedStreamConfig, crate::DefaultStreamConfigError> {
        todo!()
    }

    fn default_output_config(
        &self,
    ) -> Result<crate::SupportedStreamConfig, crate::DefaultStreamConfigError> {
        Ok(crate::SupportedStreamConfig {
            channels: 2,
            sample_rate: crate::SampleRate(44100),
            buffer_size: SupportedBufferSize::Unknown,
            sample_format: SampleFormat::I16,
        })
    }

    fn build_input_stream_raw<D, E>(
        &self,
        _config: &crate::StreamConfig,
        _sample_format: crate::SampleFormat,
        _data_callback: D,
        _error_callback: E,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Self::Stream, crate::BuildStreamError>
    where
        D: FnMut(&crate::Data, &crate::InputCallbackInfo) + Send + 'static,
        E: FnMut(crate::StreamError) + Send + 'static,
    {
        todo!()
    }

    fn build_output_stream_raw<D, E>(
        &self,
        config: &crate::StreamConfig,
        sample_format: crate::SampleFormat,
        mut data_callback: D,
        _error_callback: E,
        _timeout: Option<std::time::Duration>,
    ) -> Result<Self::Stream, crate::BuildStreamError>
    where
        D: FnMut(&mut crate::Data, &crate::OutputCallbackInfo) + Send + 'static,
        E: FnMut(crate::StreamError) + Send + 'static,
    {
        assert!(config.channels <= 2, "too many channels");
        let format = match sample_format {
            crate::SampleFormat::I8 => {
                if config.channels == 2 {
                    AudioFormat::PCM8Stereo
                } else {
                    AudioFormat::PCM8Mono
                }
            }
            crate::SampleFormat::I16 => {
                if config.channels == 2 {
                    AudioFormat::PCM16Stereo
                } else {
                    AudioFormat::PCM16Mono
                }
            }
            _ => unreachable!(),
        };
        let buf_samples = match config.buffer_size {
            crate::BufferSize::Default => (config.sample_rate.0 as f32 * 120.0 / 1000.0) as usize,
            crate::BufferSize::Fixed(_) => todo!(),
        };
        let buf_bytes = buf_samples * format.size();
        assert!(buf_bytes > 0);
        let playing = Arc::new(AtomicBool::new(false));
        let start = Instant::now();
        let config = config.clone();
        let id = self.data.streams.add_stream(buf_bytes, format, {
            let playing = playing.clone();
            move |wave_bufs| {
                if !playing.load(std::sync::atomic::Ordering::SeqCst) {
                    return None;
                }
                if wave_bufs.iter().all(|w| w.status() == wave::Status::Done) {
                    println!("all wave buffers have completed, uh oh");
                }

                let (buf_idx, WaveWrap(ref mut wave_buf)) =
                    wave_bufs.iter_mut().enumerate().find(|(_, w)| {
                        !matches!(w.0.status(), wave::Status::Playing | wave::Status::Queued)
                    })?;
                let cfg = ChannelConfig {
                    format,
                    interp: ndsp::InterpolationType::Linear,
                    sample_rate: config.sample_rate.0 as f32,
                    buf_idx,
                };

                let buf = wave_buf.get_buffer_mut().unwrap();
                assert_eq!(buf.len(), buf_bytes);

                let len = buf.len() / sample_format.sample_size();
                let mut data =
                    unsafe { Data::from_parts(buf.as_mut_ptr() as *mut _, len, sample_format) };
                let now = Instant::now();
                let elapsed = now - start;
                let timestamp = OutputStreamTimestamp {
                    callback: StreamInstant::from_nanos(elapsed.as_nanos() as i64),
                    playback: StreamInstant::from_nanos(elapsed.as_nanos() as i64),
                };

                data_callback(&mut data, &OutputCallbackInfo { timestamp });
                wave_buf.set_sample_count(buf_samples).unwrap();
                Some(cfg)
            }
        });
        Ok(super::stream::Stream {
            playing,
            host_data: self.data.clone(),
            id,
        })
    }
}
