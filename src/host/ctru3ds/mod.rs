use std::{
    ops::{Deref, DerefMut},
    pin::Pin,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, sync_channel, Receiver, Sender, SyncSender},
        Arc, Condvar, Mutex, RwLock, Weak,
    },
    thread::{JoinHandle, Thread},
    time::Duration,
};

use ctru::{
    linear::LinearAllocator,
    services::ndsp::{
        self,
        wave::{self, Wave},
        AudioFormat, InterpolationType, Ndsp,
    },
};

use crate::{traits::HostTrait, HostUnavailable};
pub use device::{Device, Devices, SupportedInputConfigs, SupportedOutputConfigs};
pub use stream::Stream;

mod device;
mod stream;

struct HostData {
    streams: Arc<StreamPool>,
    destroy_lock: Mutex<()>,
}
impl Drop for HostData {
    fn drop(&mut self) {
        println!("drop host data");
        let _l = self.destroy_lock.lock().unwrap();
        unsafe {
            ctru_sys::ndspSetCallback(None, std::ptr::null_mut());
        }
    }
}
struct ChannelConfig {
    format: AudioFormat,
    interp: InterpolationType,
    sample_rate: f32,
    buf_idx: usize,
}

pub struct Host {
    data: Pin<Arc<HostData>>,
}

unsafe extern "C" fn frame_callback<F>(data: *mut std::ffi::c_void)
where
    F: FnMut() -> bool + 'static,
{
    let mut cb = Box::from_raw(
        NonNull::new(data as *mut F)
            .expect("data pointer to frame_callback cannot be null")
            .as_ptr(),
    );
    if (*cb)() {
        Box::leak(cb);
    } else {
        drop(cb);
        ctru_sys::ndspSetCallback(None, std::ptr::null_mut());
    }
}

fn set_ndsp_callback<F: FnMut() -> bool + 'static>(_ndsp: &mut Ndsp, callback: F) {
    unsafe {
        ctru_sys::ndspSetCallback(
            Some(frame_callback::<F>),
            (Box::leak(Box::new(callback)) as *mut F).cast(),
        );
    }
}

impl Host {
    pub fn new() -> Result<Self, HostUnavailable> {
        let data = Pin::new(Arc::new_cyclic(|data: &Weak<HostData>| {
            let streams = StreamPool::new({
                let data = data.clone();
                move |ndsp| {
                    set_ndsp_callback(ndsp, move || {
                        let Some(data) = data.upgrade() else {
                            // make sure cleanup happens
                            return false;
                        };
                        let Ok(_l) = data.destroy_lock.try_lock() else {
                            // make sure cleanup happens
                            return false;
                        };
                        data.streams.tick();
                        true
                    });
                }
            });

            HostData {
                streams,
                destroy_lock: Default::default(),
            }
        }));
        Ok(Self { data })
    }
}
impl HostTrait for Host {
    type Devices = Devices;
    type Device = device::Device;

    fn is_available() -> bool {
        true
    }

    fn devices(&self) -> Result<Self::Devices, crate::DevicesError> {
        Ok(Devices(Some(Device::new(self.data.clone()))))
    }

    fn default_input_device(&self) -> Option<Self::Device> {
        None
    }

    fn default_output_device(&self) -> Option<Self::Device> {
        self.output_devices().ok().and_then(|mut d| d.next())
    }
}

const NB_WAVE_BUFFERS: usize = 2;
const MAX_CHANNEL: u8 = 24;

type StreamCallback = dyn FnMut(&mut [WaveWrap]) -> Option<ChannelConfig> + Send;

struct WaveWrap(wave::Wave);
unsafe impl Send for WaveWrap {}

impl Deref for WaveWrap {
    type Target = wave::Wave;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for WaveWrap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

struct CallbackBlock {
    id: usize,
    buffers: [WaveWrap; NB_WAVE_BUFFERS],
    cb: Arc<Mutex<StreamCallback>>,
}

struct ConfigBlock {
    callback_idx: usize,
    channel_idx: u8,
    cfg: ChannelConfig,
}

struct StreamPool {
    callbacks: Mutex<Vec<CallbackBlock>>,
    next_id: AtomicUsize,
    data_thread: Option<JoinHandle<()>>,
    cfg_tx: SyncSender<ConfigBlock>,
}

impl StreamPool {
    fn new(on_init: impl FnOnce(&mut Ndsp) + Send + 'static) -> Arc<Self> {
        let (cfg_tx, cfg_rx) = sync_channel(NB_WAVE_BUFFERS);
        Arc::new_cyclic(move |me: &Weak<Self>| Self {
            callbacks: Default::default(),
            next_id: Default::default(),
            data_thread: Some(std::thread::spawn({
                let me_w = me.clone();
                move || {
                    let mut ndsp = Ndsp::new().unwrap();
                    on_init(&mut ndsp);
                    std::thread::park();
                    'l: loop {
                        let Some(me) = me_w.upgrade() else {
                            break 'l;
                        };
                        me.tick_data();
                        // we need to make sure to not hold this upgraded weak across the potential park because
                        // otherwise we may never be unparked and get a reference cycle

                        while let Ok(ConfigBlock {
                            callback_idx,
                            cfg,
                            channel_idx,
                        }) = cfg_rx.try_recv()
                        {
                            let mut chan = ndsp.channel(channel_idx).unwrap();
                            let mut block = me.callbacks.lock().unwrap();
                            let block: &mut CallbackBlock = &mut block[callback_idx];
                            let block = &mut block.buffers[cfg.buf_idx];
                            chan.set_format(cfg.format);
                            chan.set_interpolation(cfg.interp);
                            chan.set_sample_rate(cfg.sample_rate);
                            chan.queue_wave(block).unwrap();
                        }
                        drop(me);
                        std::thread::park();
                    }
                }
            })),
            cfg_tx,
        })
    }

    fn add_stream(
        &self,
        buf_bytes: usize,
        format: AudioFormat,
        run: impl FnMut(&mut [WaveWrap]) -> Option<ChannelConfig> + Send + 'static,
    ) -> usize {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.callbacks.lock().unwrap().push(CallbackBlock {
            id,
            buffers: std::array::from_fn(|_| {
                WaveWrap(wave::Wave::new(
                    {
                        let mut v = Vec::with_capacity_in(buf_bytes, LinearAllocator);
                        v.resize(buf_bytes, 0);
                        v.into_boxed_slice()
                    },
                    format,
                    false,
                ))
            }),
            cb: Arc::new(Mutex::new(run)),
        });
        id
    }
    fn remove_stream(&self, id: usize) {
        let mut callbacks = self.callbacks.lock().unwrap();
        let idx = callbacks
            .iter()
            .position(|c| c.id == id)
            .expect("tried to remove stream with non-existant id");
        callbacks.swap_remove(idx);
    }

    fn tick_data(&self) {
        // todo: scheduler to pick free channels
        let mut chan_id = 0;
        let mut cbs = self.callbacks.lock().unwrap();
        for (callback_idx, block) in cbs.iter_mut().enumerate() {
            if let Some(cfg) = (block.cb.lock().unwrap())(&mut block.buffers) {
                self.cfg_tx
                    .send(ConfigBlock {
                        callback_idx,
                        channel_idx: chan_id,
                        cfg,
                    })
                    .expect("failed to send config block to thread... how");
            }
            chan_id = (chan_id + 1) % MAX_CHANNEL;
        }
    }

    fn tick(&self) {
        self.data_thread.as_ref().unwrap().thread().unpark();
    }
}
impl Drop for StreamPool {
    fn drop(&mut self) {
        let t = self.data_thread.take().unwrap();
        t.thread().unpark();
        t.join().unwrap();
    }
}
