use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::processing::{self, palettes::Palette, FrameResult};
use crate::usb_stream::UsbStream;

use super::types::*;

pub struct CameraAcquisition {
    stream: Arc<UsbStream>,
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
    inverted: Arc<AtomicBool>,
}

unsafe impl Send for CameraAcquisition {}

impl CameraAcquisition {
    pub fn new(stream: Arc<UsbStream>) -> Self {
        Self {
            stream,
            streaming: Arc::new(AtomicBool::new(false)),
            current_palette: Arc::new(Mutex::new(Palette::IronBlack)),
            inverted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    pub fn set_inverted(&self, invert: bool) {
        self.inverted.store(invert, Ordering::Relaxed);
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    pub fn start_stream<F>(&self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(FrameResult) + Send + Sync + 'static,
    {
        let palette = self.current_palette.clone();
        let inverted = self.inverted.clone();

        self.stream.start_stream(move |y16_data, width, height| {
            let current_palette = *palette.lock();
            let invert = inverted.load(Ordering::Relaxed);
            let result = processing::process_frame(y16_data, width as usize, height as usize, current_palette, invert);
            on_frame(result);
        })?;

        self.streaming.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop_stream(&self) {
        if self.streaming.load(Ordering::Relaxed) {
            let _ = self.stream.stop_stream();
            self.streaming.store(false, Ordering::Relaxed);
        }
    }
}
