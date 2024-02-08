use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use ctru::{
    linear::LinearAllocator,
    services::ndsp::{self, wave, AudioFormat, Ndsp},
};

use crate::{
    host::ctru3ds::DEFAULT_BUFFER_SIZE,
    traits::{DeviceTrait, StreamTrait},
    Data, OutputCallbackInfo, OutputStreamTimestamp, SampleFormat, StreamInstant,
    SupportedBufferSize, SupportedInputConfigs, SupportedOutputConfigs,
};

use super::StreamPool;

pub struct Device {
    streams: Arc<StreamPool>,
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
        Ok(std::iter::once(self.default_output_config()?))
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
        data_callback: D,
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
        let wave_sz = buf_sz * config.channels * sample_format.sample_size();
        assert!(wave_sz > 0);
        let playing = Arc::new(AtomicBool::new(false));
        let wave_buf = wave::Wave::new(
            {
                let mut v = Vec::with_capacity_in(wave_sz, LinearAllocator);
                v.resize(wave_sz, 0);
                v.into_boxed_slice()
            },
            format,
            false,
        );
        let start = Instant::now();
        let nb_chans = config.channels as usize;
        let id = self.streams.add_stream(move |chan: &ndsp::Channel| {
            if !playing.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if wave_buf.status() != wave::Status::Done {
                return;
            }
            chan.set_format(format);
            chan.set_interpolation(ndsp::InterpolationType::Linear);
            chan.set_sample_rate(config.sample_rate.0);

            let mut buf = wave.get_buffer_mut().unwrap();

            let len = buf.len();
            let mut data = unsafe { Data::from_parts(&mut buf, len, sample_format) };
            let now = Instant::now();
            let elapsed = now - start;
            let timestamp = OutputStreamTimestamp {
                callback: StreamInstant::from_nanos(elapsed.as_nanos()),
                playback: StreamInstant::from_nanos(elapsed.as_nanos()),
            };
            data_callback(&mut data, &OutputCallbackInfo { timestamp });

            chan.queue_wave(&mut wave_buf);
        });
        Ok(super::stream::Stream {
            paused,
            pool: self.streams.clone(),
            id,
        })
    }
}
