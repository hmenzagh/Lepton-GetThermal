use std::ffi::CStr;
use std::os::raw::c_int;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use parking_lot::Mutex;

use crate::processing::{self, palettes::Palette, FrameResult};
use crate::uvc_ffi::*;

use super::types::*;

/// Manages UVC device connection and frame streaming.
#[allow(dead_code)]
pub struct CameraAcquisition {
    ctx: *mut uvc_context,
    dev: *mut uvc_device,
    devh: *mut uvc_device_handle,
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
    frame_width: u32,
    frame_height: u32,
}

// Raw pointers are Send-safe in our usage (single-owner with mutex)
unsafe impl Send for CameraAcquisition {}

#[allow(dead_code)]
impl CameraAcquisition {
    /// Initialize UVC context and find PureThermal device.
    pub fn connect() -> Result<Self, CameraError> {
        unsafe {
            let mut ctx: *mut uvc_context = std::ptr::null_mut();
            let ret = uvc_init(&mut ctx, std::ptr::null_mut());
            if ret != UVC_SUCCESS {
                return Err(CameraError::UvcError(format!("uvc_init failed: {ret}")));
            }

            let mut dev: *mut uvc_device = std::ptr::null_mut();
            let ret = uvc_find_device(
                ctx,
                &mut dev,
                PT_VID as c_int,
                PT_PID as c_int,
                std::ptr::null(),
            );
            if ret != UVC_SUCCESS {
                uvc_exit(ctx);
                return Err(CameraError::DeviceNotFound);
            }

            let mut devh: *mut uvc_device_handle = std::ptr::null_mut();
            let ret = uvc_open(dev, &mut devh);
            if ret != UVC_SUCCESS {
                let msg = CStr::from_ptr(uvc_strerror(ret))
                    .to_string_lossy()
                    .into_owned();
                uvc_unref_device(dev);
                uvc_exit(ctx);
                return Err(CameraError::OpenFailed(msg));
            }

            Ok(Self {
                ctx,
                dev,
                devh,
                streaming: Arc::new(AtomicBool::new(false)),
                current_palette: Arc::new(Mutex::new(Palette::IronBlack)),
                frame_width: 0,
                frame_height: 0,
            })
        }
    }

    /// Returns raw device handle for Lepton SDK commands.
    pub fn device_handle(&self) -> *mut uvc_device_handle {
        self.devh
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    /// Start streaming Y16 frames. Calls `on_frame` for each processed frame.
    pub fn start_stream<F>(
        &mut self,
        width: i32,
        height: i32,
        fps: i32,
        on_frame: F,
    ) -> Result<(), CameraError>
    where
        F: Fn(FrameResult) + Send + 'static,
    {
        unsafe {
            let mut ctrl = StreamCtrlBuf::zeroed();
            let ret = uvc_get_stream_ctrl_format_size(
                self.devh,
                ctrl.as_ptr(),
                UvcFrameFormat::Y16 as std::os::raw::c_uint,
                width as std::os::raw::c_int,
                height as std::os::raw::c_int,
                fps as std::os::raw::c_int,
            );
            if ret != UVC_SUCCESS {
                return Err(CameraError::StreamFailed(format!(
                    "format negotiation failed: {ret}"
                )));
            }

            self.frame_width = width as u32;
            self.frame_height = height as u32;

            // Bundle callback data
            let palette = self.current_palette.clone();
            let w = width as usize;
            let h = height as usize;

            let callback_data = Box::new(CallbackData {
                on_frame: Box::new(on_frame),
                palette,
                width: w,
                height: h,
            });
            let user_ptr = Box::into_raw(callback_data) as *mut std::ffi::c_void;

            let ret = uvc_start_streaming(self.devh, ctrl.as_ptr(), frame_callback, user_ptr, 0);
            if ret != UVC_SUCCESS {
                // Clean up leaked callback_data
                let _ = Box::from_raw(user_ptr as *mut CallbackData);
                return Err(CameraError::StreamFailed(format!(
                    "start_streaming failed: {ret}"
                )));
            }

            self.streaming.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    pub fn stop_stream(&mut self) {
        if self.streaming.load(Ordering::Relaxed) {
            unsafe {
                uvc_stop_streaming(self.devh);
            }
            self.streaming.store(false, Ordering::Relaxed);
        }
    }
}

impl Drop for CameraAcquisition {
    fn drop(&mut self) {
        self.stop_stream();
        unsafe {
            if !self.devh.is_null() {
                uvc_close(self.devh);
            }
            if !self.dev.is_null() {
                uvc_unref_device(self.dev);
            }
            if !self.ctx.is_null() {
                uvc_exit(self.ctx);
            }
        }
    }
}

struct CallbackData {
    on_frame: Box<dyn Fn(FrameResult) + Send>,
    palette: Arc<Mutex<Palette>>,
    width: usize,
    height: usize,
}

unsafe extern "C" fn frame_callback(frame: *mut uvc_frame, user_ptr: *mut std::ffi::c_void) {
    let data = &*(user_ptr as *const CallbackData);
    let frame_ref = &*frame;

    let y16_slice = std::slice::from_raw_parts(frame_ref.data, frame_ref.data_bytes);

    let palette = *data.palette.lock();
    let result = processing::process_frame(y16_slice, data.width, data.height, palette);

    (data.on_frame)(result);
}
