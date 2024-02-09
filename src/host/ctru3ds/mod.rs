#![feature(allocator_api)]

use std::{
    ops::DerefMut,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Sender},
        Arc, Condvar, Mutex, RwLock, Weak,
    },
    thread::{JoinHandle, Thread},
};

use ctru::{
    linear::LinearAllocator,
    services::ndsp::{self, wave, AudioFormat, Ndsp},
};

use crate::{traits::HostTrait, HostUnavailable};
pub use device::{Device, Devices, SupportedInputConfigs, SupportedOutputConfigs};
pub use stream::Stream;

mod device;
mod stream;

struct HostData {
    inst: Arc<Mutex<Ndsp>>,
    streams: Arc<StreamPool>,
}
impl Drop for HostData {
    fn drop(&mut self) {
        println!("drop host data");
        unsafe {
            ctru_sys::ndspSetCallback(None, std::ptr::null_mut());
        }
    }
}

pub struct Host {
    data: Pin<Arc<HostData>>,
}

unsafe extern "C" fn frame_callback(data: *mut std::ffi::c_void) {
    let data = data.cast_const().cast::<HostData>().as_ref().unwrap();
    data.streams.tick();
}

impl Host {
    pub fn new() -> Result<Self, HostUnavailable> {
        let inst = Arc::new(Mutex::new(Ndsp::new().map_err(|e| match e {
            ctru::Error::ServiceAlreadyActive => panic!("already initialised Ndsp"),
            ctru::Error::Os(o) => panic!("os: {o}"),
            ctru::Error::Libc(s) => panic!("libc: {s}"),
            _ => HostUnavailable,
        })?));
        let streams = StreamPool::new(inst.clone());
        let data = Arc::pin(HostData { inst, streams });
        unsafe { ctru_sys::ndspSetCallback(Some(frame_callback), (&(*data) as *const _) as *mut _) }
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

const NB_WAVE_BUFFERS: usize = 5;
const DECODE_FRAMES_AHEAD: usize = 2;
const MAX_CHANNEL: usize = 24;

type StreamCallback = dyn FnMut(&mut ndsp::Channel) + Send;
type TickCallback = dyn FnMut() + Send;

struct CallbackBlock {
    id: usize,
    cb: Arc<Mutex<StreamCallback>>,
}

struct StreamPool {
    ndsp: NdspWrap,
    callbacks: Mutex<Vec<CallbackBlock>>,
    next_id: AtomicUsize,
    data_thread: JoinHandle<()>,
}

impl StreamPool {
    fn new(ndsp: Arc<Mutex<Ndsp>>) -> Arc<Self> {
        Arc::new_cyclic(|me: &Weak<Self>| Self {
            callbacks: Default::default(),
            next_id: Default::default(),
            ndsp: NdspWrap(ndsp),
            data_thread: std::thread::spawn({
                let me = me.clone();
                move || {
                    std::thread::park();
                    loop {
                        let Some(me) = me.upgrade() else {
                            break;
                        };
                        me.tick_data();
                        std::thread::park();
                    }
                }
            }),
        })
    }

    fn add_stream(&self, run: impl FnMut(&mut ndsp::Channel) + Send + 'static) -> usize {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.callbacks.lock().unwrap().push(CallbackBlock {
            id,
            cb: Arc::new(Mutex::new(run)),
        });
        id
    }
    fn remove_stream(&self, id: usize) {
        println!("remove stream");
        let mut callbacks = self.callbacks.lock().unwrap();
        let idx = callbacks
            .iter()
            .position(|c| c.id == id)
            .expect("tried to remove stream with non-existant id");
        callbacks.swap_remove(idx);
    }

    fn tick_data(&self) {
        // todo: scheduler to pick free channels
        let inst = self.ndsp.0.lock().unwrap();
        let mut chan_id = 0;
        let mut cbs = self.callbacks.lock().unwrap();
        for block in cbs.iter_mut() {
            let mut chan = inst.channel(chan_id as u8).unwrap();
            (block.cb.lock().unwrap())(&mut chan);
            chan_id = (chan_id + 1) % MAX_CHANNEL;
        }
    }

    fn tick(&self) {
        self.data_thread.thread().unpark();
    }
}

struct NdspWrap(Arc<Mutex<Ndsp>>);

unsafe impl Send for NdspWrap {}
unsafe impl Sync for NdspWrap {}
