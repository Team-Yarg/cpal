#![feature(allocator_api)]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex, RwLock,
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

pub struct Host {
    inst: Ndsp,
    streams: Arc<StreamPool>,
}

impl Host {
    pub fn new() -> Result<Self, HostUnavailable> {
        Ok(Self {
            inst: Ndsp::new().map_err(|_| HostUnavailable)?,
            streams: Default::default(),
        })
    }
}
impl HostTrait for Host {
    type Devices = Devices;
    type Device = device::Device;

    fn is_available() -> bool {
        true
    }

    fn devices(&self) -> Result<Self::Devices, crate::DevicesError> {
        Ok(Devices(Some(Device::new(self.streams.clone()))))
    }

    fn default_input_device(&self) -> Option<Self::Device> {
        None
    }

    fn default_output_device(&self) -> Option<Self::Device> {
        self.output_devices().ok().and_then(|mut d| d.next())
    }
}

const MAX_CHANNEL: usize = 24;

type StreamCallback = dyn FnMut(&mut ndsp::Channel);

struct CallbackBlock {
    id: usize,
    cb: Box<StreamCallback>,
}

#[derive(Default)]
struct StreamPool {
    callbacks: Mutex<Vec<CallbackBlock>>,
    next_id: AtomicUsize,
}

impl StreamPool {
    fn add_stream(&self, cb: impl FnMut(&mut ndsp::Channel) + 'static) -> usize {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.callbacks.lock().unwrap().push(CallbackBlock {
            id,
            cb: Box::new(cb),
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

    fn tick(&self, inst: &Ndsp) {
        // todo: scheduler to pick free channels
        let mut chan_id = 0;
        let mut cbs = self.callbacks.lock().unwrap();
        for block in cbs.iter_mut() {
            let mut chan = inst.channel(chan_id as u8).unwrap();
            (block.cb)(&mut chan);
            chan_id = (chan_id + 1) % MAX_CHANNEL;
        }
    }
}
