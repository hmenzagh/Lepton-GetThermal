use std::sync::{
    atomic::{AtomicBool, AtomicU16, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::processing::{self, palettes::Palette, upscale::Upscaler, FrameResult};
use crate::usb_stream::UsbStream;

use super::types::*;

pub struct CameraAcquisition {
    stream: Arc<UsbStream>,
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
    inverted: Arc<AtomicBool>,
    /// Raw Y16 threshold for isotherm overlay. 0 = disabled.
    isotherm_raw: Arc<AtomicU16>,
    /// Neural network upscaler (None = disabled).
    upscaler: Arc<Mutex<Option<Upscaler>>>,
    /// Whether upscaling is enabled by the user.
    upscale_enabled: Arc<AtomicBool>,
}

unsafe impl Send for CameraAcquisition {}

impl CameraAcquisition {
    pub fn new(stream: Arc<UsbStream>) -> Self {
        Self {
            stream,
            streaming: Arc::new(AtomicBool::new(false)),
            current_palette: Arc::new(Mutex::new(Palette::IronBlack)),
            inverted: Arc::new(AtomicBool::new(false)),
            isotherm_raw: Arc::new(AtomicU16::new(0)),
            upscaler: Arc::new(Mutex::new(None)),
            upscale_enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    pub fn set_inverted(&self, invert: bool) {
        self.inverted.store(invert, Ordering::Relaxed);
    }

    pub fn set_isotherm_raw(&self, threshold: u16) {
        self.isotherm_raw.store(threshold, Ordering::Relaxed);
    }

    /// Enable or disable neural network upscaling.
    /// Lazily initializes the ONNX model on first enable.
    pub fn set_upscale(&self, enabled: bool) {
        if enabled {
            let mut guard = self.upscaler.lock();
            if guard.is_none() {
                match Upscaler::new() {
                    Ok(up) => {
                        eprintln!("[thermal-v2] Upscaler initialized ({}x)", up.scale);
                        *guard = Some(up);
                    }
                    Err(e) => {
                        eprintln!("[thermal-v2] Failed to init upscaler: {e}");
                        return;
                    }
                }
            }
        }
        self.upscale_enabled.store(enabled, Ordering::Relaxed);
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
        let isotherm_raw = self.isotherm_raw.clone();
        let upscaler = self.upscaler.clone();
        let upscale_enabled = self.upscale_enabled.clone();

        self.stream.start_stream(move |y16_data, width, height| {
            let current_palette = *palette.lock();
            let invert = inverted.load(Ordering::Relaxed);
            let iso = isotherm_raw.load(Ordering::Relaxed);

            let mut up_guard = upscaler.lock();
            let up_ref = if upscale_enabled.load(Ordering::Relaxed) {
                up_guard.as_mut()
            } else {
                None
            };

            let result = processing::process_frame(
                y16_data,
                width as usize,
                height as usize,
                current_palette,
                invert,
                iso,
                up_ref,
            );
            drop(up_guard);
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
