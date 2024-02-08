use std::sync::atomic::AtomicBool;

use ctru::services::ndsp;

use super::StreamPool;

pub struct Stream {
    pub paused: Arc<AtomicBool>,
    pub pool: Arc<StreamPool>,
    pub id: usize,
}

impl StreamTrait for Stream {
    fn play(&self) -> Result<(), crate::PlayStreamError> {
        self.paused.set()
    }

    fn pause(&self) -> Result<(), crate::PauseStreamError> {
        self.chan.set_paused(true);
        Ok(())
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        self.pool.remove_stream(self.id);
    }
}
