use std::sync::{atomic::AtomicBool, Arc};

use ctru::services::ndsp;

use crate::traits::StreamTrait;

use super::StreamPool;

pub struct Stream {
    pub playing: Arc<AtomicBool>,
    pub pool: Arc<StreamPool>,
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
        self.pool.remove_stream(self.id);
    }
}
