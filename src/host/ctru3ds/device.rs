use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use ctru::{
    linear::LinearAllocator,
    services::ndsp::{self, wave, AudioFormat, Ndsp},
};

use crate::{
    traits::{DeviceTrait, StreamTrait},
    Data, OutputCallbackInfo, OutputStreamTimestamp, SampleFormat, StreamInstant,
    SupportedBufferSize, SupportedStreamConfig, SupportedStreamConfigRange,
};

pub type SupportedInputConfigs = std::iter::Once<SupportedStreamConfigRange>;
pub type SupportedOutputConfigs = std::iter::Once<SupportedStreamConfigRange>;

use super::StreamPool;

#[derive(Clone)]
pub struct Device {
    streams: Arc<StreamPool>,
}
pub struct Devices(pub Option<Device>);
impl Iterator for Devices {
    type Item = Device;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.take()
    }
}

impl Device {
    pub fn new(streams: Arc<StreamPool>) -> Self {
        Self { streams }
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
            sample_rate: crate::SampleRate(44000),
            buffer_size: SupportedBufferSize::Unknown,
            sample_format: SampleFormat::U16,
        })
    }

    fn build_input_stream_raw<D, E>(
        &self,
        config: &crate::StreamConfig,
        sample_format: crate::SampleFormat,
        data_callback: D,
        error_callback: E,
        timeout: Option<std::time::Duration>,
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
        error_callback: E,
        timeout: Option<std::time::Duration>,
    ) -> Result<Self::Stream, crate::BuildStreamError>
    where
        D: FnMut(&mut crate::Data, &crate::OutputCallbackInfo) + Send + 'static,
        E: FnMut(crate::StreamError) + Send + 'static,
    {
        assert!(config.channels <= 2, "too many channels");
        let format = match sample_format {
            crate::SampleFormat::U8 => {
                if config.channels == 2 {
                    AudioFormat::PCM8Stereo
                } else {
                    AudioFormat::PCM8Mono
                }
            }
            crate::SampleFormat::U16 => {
                if config.channels == 2 {
                    AudioFormat::PCM16Stereo
                } else {
                    AudioFormat::PCM16Mono
                }
            }
            _ => unreachable!(),
        };
        let buf_sz = match config.buffer_size {
            crate::BufferSize::Default => (config.sample_rate.0 as f32 * 120.0 / 1000.0) as usize,
            crate::BufferSize::Fixed(_) => todo!(),
        };
        let wave_sz = buf_sz * config.channels as usize * sample_format.sample_size();
        assert!(wave_sz > 0);
        let playing = Arc::new(AtomicBool::new(false));
        let mut wave_buf = wave::Wave::new(
            {
                let mut v = Vec::with_capacity_in(wave_sz, LinearAllocator);
                v.resize(wave_sz, 0);
                v.into_boxed_slice()
            },
            format,
            false,
        );
        let start = Instant::now();
        let config = config.clone();
        let id = self.streams.add_stream({
            let playing = playing.clone();
            move |chan| {
                if !playing.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                if wave_buf.status() != wave::Status::Done {
                    return;
                }
                chan.set_format(format);
                chan.set_interpolation(ndsp::InterpolationType::Linear);
                chan.set_sample_rate(config.sample_rate.0 as f32);

                let buf = wave_buf.get_buffer_mut().unwrap();

                let len = buf.len();
                let mut data =
                    unsafe { Data::from_parts(buf.as_mut_ptr() as *mut _, len, sample_format) };
                let now = Instant::now();
                let elapsed = now - start;
                let timestamp = OutputStreamTimestamp {
                    callback: StreamInstant::from_nanos(elapsed.as_nanos() as i64),
                    playback: StreamInstant::from_nanos(elapsed.as_nanos() as i64),
                };
                data_callback(&mut data, &OutputCallbackInfo { timestamp });

                chan.queue_wave(&mut wave_buf).unwrap();
            }
        });
        Ok(super::stream::Stream {
            playing,
            pool: self.streams.clone(),
            id,
        })
    }
}
