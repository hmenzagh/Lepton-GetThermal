use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::avfoundation::{AvCamera, CapturedFormat};
use crate::processing::{self, palettes::Palette, FrameResult};

use super::types::*;

pub struct CameraAcquisition {
    camera: AvCamera,
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
}

unsafe impl Send for CameraAcquisition {}

impl CameraAcquisition {
    pub fn connect() -> Result<Self, CameraError> {
        let camera = AvCamera::discover()?;
        Ok(Self {
            camera,
            streaming: Arc::new(AtomicBool::new(false)),
            current_palette: Arc::new(Mutex::new(Palette::IronBlack)),
        })
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    pub fn start_stream<F>(&mut self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(FrameResult) + Send + 'static,
    {
        let palette = self.current_palette.clone();
        let camera_format = self.camera.format();
        let w = self.camera.width();
        let h = self.camera.height();

        self.camera.start(move |captured| {
            let current_palette = *palette.lock();

            let result = match camera_format {
                CapturedFormat::Y16 => {
                    // Use our processing pipeline: Y16 -> auto-gain -> colorize -> RGBA
                    processing::process_frame(&captured.data, w, h, current_palette)
                }
                CapturedFormat::BGRA => {
                    // Camera applied its own AGC -- convert BGRA to RGBA
                    let rgba = bgra_to_rgba(&captured.data);
                    FrameResult {
                        rgba,
                        width: captured.width,
                        height: captured.height,
                        stats: processing::autogain::GainResult {
                            grayscale: Vec::new(),
                            min_val: 0,
                            max_val: 0,
                            min_pos: 0,
                            max_pos: 0,
                        },
                    }
                }
            };

            on_frame(result);
        })?;

        self.streaming.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn stop_stream(&mut self) {
        if self.streaming.load(Ordering::Relaxed) {
            self.camera.stop();
            self.streaming.store(false, Ordering::Relaxed);
        }
    }
}

fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(bgra.len());
    for chunk in bgra.chunks(4) {
        if chunk.len() == 4 {
            rgba.push(chunk[2]); // R
            rgba.push(chunk[1]); // G
            rgba.push(chunk[0]); // B
            rgba.push(chunk[3]); // A
        }
    }
    rgba
}
