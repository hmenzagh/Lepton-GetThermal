use serde::{Deserialize, Serialize};

/// PureThermal USB Vendor/Product IDs
pub const PT_VID: u16 = 0x1e4e;
pub const PT_PID: u16 = 0x0100;

/// UVC frame format identifiers (matches libuvc enum)
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum UvcFrameFormat {
    Y16 = 13,   // UVC_FRAME_FORMAT_Y16
    Rgb = 7,    // UVC_FRAME_FORMAT_RGB
    Gray8 = 11, // UVC_FRAME_FORMAT_GRAY8
}

/// Camera device information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct DeviceInfo {
    pub serial_number: String,
    pub part_number: String,
    pub firmware_version: String,
    pub supports_radiometry: bool,
    pub supports_hw_pseudo_color: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Camera connection state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(dead_code)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Streaming,
    Error(String),
}

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum CameraError {
    #[error("No PureThermal device found")]
    DeviceNotFound,
    #[error("Failed to open device: {0}")]
    OpenFailed(String),
    #[error("Failed to start stream: {0}")]
    StreamFailed(String),
    #[error("Lepton SDK error: {0}")]
    LeptonError(String),
    #[error("UVC error: {0}")]
    UvcError(String),
}
