//! Minimal safe FFI wrappers around libuvc.
//! Only exposes the functions we need for PureThermal + Lepton.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uint};

// Opaque types
pub enum uvc_context {}
pub enum uvc_device {}
pub enum uvc_device_handle {}
pub enum uvc_stream_ctrl {}

/// UVC frame delivered by streaming callback
#[repr(C)]
pub struct uvc_frame {
    pub data: *mut u8,
    pub data_bytes: usize,
    pub width: u32,
    pub height: u32,
    pub frame_format: u32,
    pub step: usize,
    pub sequence: u32,
    // Additional fields exist but we don't need them
}

pub type uvc_frame_callback_t =
    unsafe extern "C" fn(frame: *mut uvc_frame, user_ptr: *mut c_void);

// UVC error type
pub type uvc_error_t = c_int;
pub const UVC_SUCCESS: uvc_error_t = 0;

// UVC_GET_CUR for control requests
pub const UVC_GET_CUR: c_uint = 0x81;

extern "C" {
    pub fn uvc_init(ctx: *mut *mut uvc_context, usb_ctx: *mut c_void) -> uvc_error_t;

    pub fn uvc_exit(ctx: *mut uvc_context);

    pub fn uvc_find_device(
        ctx: *mut uvc_context,
        dev: *mut *mut uvc_device,
        vid: c_int,
        pid: c_int,
        sn: *const c_char,
    ) -> uvc_error_t;

    pub fn uvc_open(dev: *mut uvc_device, devh: *mut *mut uvc_device_handle) -> uvc_error_t;

    pub fn uvc_close(devh: *mut uvc_device_handle);
    pub fn uvc_unref_device(dev: *mut uvc_device);

    pub fn uvc_get_stream_ctrl_format_size(
        devh: *mut uvc_device_handle,
        ctrl: *mut uvc_stream_ctrl,
        format: c_uint,
        width: c_int,
        height: c_int,
        fps: c_int,
    ) -> uvc_error_t;

    pub fn uvc_start_streaming(
        devh: *mut uvc_device_handle,
        ctrl: *mut uvc_stream_ctrl,
        cb: uvc_frame_callback_t,
        user_ptr: *mut c_void,
        flags: u8,
    ) -> uvc_error_t;

    pub fn uvc_stop_streaming(devh: *mut uvc_device_handle);

    // Extension unit control access (for Lepton SDK)
    pub fn uvc_get_ctrl(
        devh: *mut uvc_device_handle,
        unit_id: u8,
        ctrl_id: u8,
        data: *mut u8,
        len: c_int,
        req_code: c_uint,
    ) -> c_int;

    pub fn uvc_set_ctrl(
        devh: *mut uvc_device_handle,
        unit_id: u8,
        ctrl_id: u8,
        data: *mut u8,
        len: c_int,
    ) -> c_int;

    pub fn uvc_strerror(err: uvc_error_t) -> *const c_char;
}

/// Opaque buffer for uvc_stream_ctrl (actual struct size varies by libuvc version).
/// We use a generous upper bound to ensure we never undersize.
pub const STREAM_CTRL_SIZE: usize = 256;

/// Aligned buffer type for uvc_stream_ctrl allocation.
#[repr(C, align(8))]
pub struct StreamCtrlBuf {
    pub data: [u8; STREAM_CTRL_SIZE],
}

impl StreamCtrlBuf {
    pub fn zeroed() -> Self {
        Self {
            data: [0u8; STREAM_CTRL_SIZE],
        }
    }
    pub fn as_ptr(&mut self) -> *mut uvc_stream_ctrl {
        self.data.as_mut_ptr() as *mut uvc_stream_ctrl
    }
}
