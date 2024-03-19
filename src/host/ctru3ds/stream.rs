use std::{
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
};

use crate::traits::StreamTrait;

use super::HostData;

pub struct Stream {
    pub playing: Arc<AtomicBool>,
    pub host_data: Pin<Arc<HostData>>,
    pub id: usize,
}

impl StreamTrait for Stream {
    fn play(&self) -> Result<(), crate::PlayStreamError> {
        self.playing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn pause(&self) -> Result<(), crate::PauseStreamError> {
        self.playing
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.host_data.streams.remove_stream(self.id);
    }
}
