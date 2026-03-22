use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::processing::{palettes::Palette, FrameResult};

use super::types::*;

pub struct CameraAcquisition {
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
}

unsafe impl Send for CameraAcquisition {}

impl CameraAcquisition {
    pub fn connect() -> Result<Self, CameraError> {
        todo!("Reimplemented in Task 5 with AVFoundation")
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    pub fn start_stream<F>(
        &mut self,
        _width: i32,
        _height: i32,
        _fps: i32,
        _on_frame: F,
    ) -> Result<(), CameraError>
    where
        F: Fn(FrameResult) + Send + 'static,
    {
        todo!("Reimplemented in Task 5 with AVFoundation")
    }

    pub fn stop_stream(&mut self) {
        // no-op stub
    }
}
