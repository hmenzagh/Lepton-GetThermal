//! AVFoundation camera capture bridge for macOS.
//!
//! Discovers PureThermal devices (which macOS sees as UVC cameras via the
//! kernel's built-in `com.apple.UVCService` driver), starts an
//! `AVCaptureSession`, and delivers frames through a Rust callback.
//!
//! Using AVFoundation instead of libuvc/libusb avoids fighting the kernel
//! driver for interface ownership (which can cause kernel panics on macOS).

use std::ptr::NonNull;
use std::sync::Arc;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, ClassType, DefinedClass};

use objc2_av_foundation::{
    AVCaptureDevice, AVCaptureDeviceDiscoverySession,
    AVCaptureDeviceInput, AVCaptureDevicePosition, AVCaptureDeviceTypeExternal,
    AVCaptureOutput, AVCaptureSession, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    kCVPixelFormatType_16Gray, kCVPixelFormatType_32BGRA, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockFlags,
};
use objc2_foundation::{NSArray, NSNumber, NSObjectProtocol, NSString};

use dispatch2::DispatchQueue;

use crate::camera::types::CameraError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Pixel format received from AVFoundation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::upper_case_acronyms)]
pub enum CapturedFormat {
    /// Raw 16-bit grayscale (ideal for radiometry).
    Y16,
    /// 32-bit BGRA (camera's built-in AGC applied).
    BGRA,
}

/// A single frame captured from AVFoundation.
pub struct CapturedFrame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// The pixel format of this frame (Y16 or BGRA).
    #[allow(dead_code)]
    pub format: CapturedFormat,
}

// ---------------------------------------------------------------------------
// Frame callback wrapper (shared between Rust and ObjC delegate)
// ---------------------------------------------------------------------------

/// Thread-safe wrapper that stores the Rust frame callback.
///
/// Wrapped in `Arc` so the delegate and `AvCamera` can both hold a reference.
/// The inner `parking_lot::Mutex` is used because the delegate callback fires
/// on a dispatch queue (different thread from the main/Tauri thread).
struct FrameCallbackHolder {
    #[allow(clippy::type_complexity)]
    callback: parking_lot::Mutex<Option<Box<dyn Fn(CapturedFrame) + Send>>>,
}

// SAFETY: The Mutex ensures synchronized access from the dispatch queue thread.
unsafe impl Send for FrameCallbackHolder {}
unsafe impl Sync for FrameCallbackHolder {}

// ---------------------------------------------------------------------------
// Objective-C delegate class
// ---------------------------------------------------------------------------

/// Ivars stored inside the ObjC delegate object.
struct DelegateIvars {
    holder: Arc<FrameCallbackHolder>,
}

define_class!(
    // SAFETY:
    // - NSObject does not have subclassing requirements we violate.
    // - FrameDelegate does not implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = DelegateIvars]
    struct FrameDelegate;

    impl FrameDelegate {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(DelegateIvars {
                holder: Arc::new(FrameCallbackHolder {
                    callback: parking_lot::Mutex::new(None),
                }),
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for FrameDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output_did_output(
            &self,
            _output: &AVCaptureOutput,
            sample_buffer: &CMSampleBuffer,
            _connection: &objc2_av_foundation::AVCaptureConnection,
        ) {
            // Extract pixel buffer from the sample buffer.
            let pixel_buffer = match unsafe { sample_buffer.image_buffer() } {
                Some(pb) => pb,
                None => return,
            };

            // Lock the pixel buffer for read-only CPU access.
            unsafe {
                CVPixelBufferLockBaseAddress(
                    &pixel_buffer,
                    CVPixelBufferLockFlags::ReadOnly,
                );
            }

            // Read dimensions and pixel data.
            let width = CVPixelBufferGetWidth(&pixel_buffer);
            let height = CVPixelBufferGetHeight(&pixel_buffer);
            let bytes_per_row = CVPixelBufferGetBytesPerRow(&pixel_buffer);
            let base = CVPixelBufferGetBaseAddress(&pixel_buffer);
            let pf = CVPixelBufferGetPixelFormatType(&pixel_buffer);

            let format = if pf == kCVPixelFormatType_16Gray {
                CapturedFormat::Y16
            } else {
                CapturedFormat::BGRA
            };

            let bytes_per_pixel: usize = match format {
                CapturedFormat::Y16 => 2,
                CapturedFormat::BGRA => 4,
            };

            eprintln!("[thermal-v2] AVF delegate: frame {}x{}, format={:?}, base_null={}, bpr={}", width, height, format, base.is_null(), bytes_per_row);

            if !base.is_null() && width > 0 && height > 0 {
                // Copy pixel data row-by-row (bytes_per_row may include padding).
                let row_data_len = width * bytes_per_pixel;
                let mut data = Vec::with_capacity(row_data_len * height);
                for row in 0..height {
                    let row_ptr = unsafe { (base as *const u8).add(row * bytes_per_row) };
                    let row_slice = unsafe { std::slice::from_raw_parts(row_ptr, row_data_len) };
                    data.extend_from_slice(row_slice);
                }

                let frame = CapturedFrame {
                    data,
                    width,
                    height,
                    format,
                };

                // Deliver frame to Rust callback.
                let guard = self.ivars().holder.callback.lock();
                if let Some(ref cb) = *guard {
                    cb(frame);
                }
            }

            // Unlock the pixel buffer.
            unsafe {
                CVPixelBufferUnlockBaseAddress(
                    &pixel_buffer,
                    CVPixelBufferLockFlags::ReadOnly,
                );
            }
        }
    }
);

impl FrameDelegate {
    /// Create a new delegate instance.
    fn new() -> Retained<Self> {
        unsafe { msg_send![Self::class(), new] }
    }
}

// ---------------------------------------------------------------------------
// CVPixelBuffer lock/unlock bindings
// ---------------------------------------------------------------------------

extern "C-unwind" {
    fn CVPixelBufferLockBaseAddress(
        pixel_buffer: &objc2_core_video::CVPixelBuffer,
        lock_flags: CVPixelBufferLockFlags,
    ) -> objc2_core_video::CVReturn;

    fn CVPixelBufferUnlockBaseAddress(
        pixel_buffer: &objc2_core_video::CVPixelBuffer,
        unlock_flags: CVPixelBufferLockFlags,
    ) -> objc2_core_video::CVReturn;
}

// ---------------------------------------------------------------------------
// AvCamera -- public API
// ---------------------------------------------------------------------------

/// AVFoundation-based camera capture for PureThermal devices.
///
/// Usage:
/// ```ignore
/// let mut cam = AvCamera::discover()?;
/// cam.start(|frame| {
///     // process frame.data (BGRA or Y16)
/// })?;
/// // ...
/// cam.stop();
/// ```
pub struct AvCamera {
    /// Retained to keep the device alive for the capture session.
    #[allow(dead_code)]
    device: Retained<AVCaptureDevice>,
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureVideoDataOutput>,
    delegate: Retained<FrameDelegate>,
    callback_holder: Arc<FrameCallbackHolder>,
    /// Dispatch queue for frame delivery — must be kept alive while streaming.
    capture_queue: Option<dispatch2::DispatchRetained<DispatchQueue>>,
    format: CapturedFormat,
    width: usize,
    height: usize,
    running: bool,
}

// SAFETY: The AVFoundation objects are used through dispatch queues (thread-
// safe). The callback holder is behind Arc<Mutex>. The `running` flag is only
// mutated from the owner thread (start/stop calls).
unsafe impl Send for AvCamera {}

impl AvCamera {
    /// Discover a PureThermal camera attached via USB.
    ///
    /// Searches for an external UVC device whose `localizedName` contains
    /// "PureThermal" (case-insensitive comparison).
    pub fn discover() -> Result<Self, CameraError> {
        // Build a discovery session for external (USB/UVC) video devices.
        // SAFETY: These are well-known Apple framework extern statics, always valid on macOS.
        let device_type_external = unsafe { AVCaptureDeviceTypeExternal };
        let media_type = unsafe { AVMediaTypeVideo }
            .expect("AVMediaTypeVideo not available on this platform");

        let device_types: Retained<NSArray<NSString>> =
            NSArray::from_slice(&[device_type_external]);

        let discovery = unsafe {
            AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
                &device_types,
                Some(media_type),
                AVCaptureDevicePosition::Unspecified,
            )
        };

        let devices = unsafe { discovery.devices() };

        // Find the PureThermal device.
        let mut found_device: Option<Retained<AVCaptureDevice>> = None;
        let count = devices.count();
        for i in 0..count {
            let d = devices.objectAtIndex(i);
            let name = unsafe { d.localizedName() };
            let name_str = name.to_string();
            if name_str.to_lowercase().contains("purethermal") {
                found_device = Some(d);
                break;
            }
        }

        let device = found_device.ok_or(CameraError::DeviceNotFound)?;
        let device_name = unsafe { device.localizedName() }.to_string();

        // Create capture session.
        let session = unsafe { AVCaptureSession::new() };

        // Create input from the discovered device.
        let input = unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&device) }
            .map_err(|e| {
                CameraError::OpenFailed(format!(
                    "Failed to create device input for {}: {}",
                    device_name,
                    e.localizedDescription()
                ))
            })?;

        // Create video data output.
        let output = unsafe { AVCaptureVideoDataOutput::new() };

        // Configure output pixel format.
        // Try to request 16-bit gray (Y16) first; fall back to BGRA.
        let (pixel_format, captured_format) = Self::choose_pixel_format(&output);

        // Set the video settings with the chosen pixel format.
        Self::set_output_pixel_format(&output, pixel_format);

        // Discard late frames to avoid memory buildup.
        unsafe { output.setAlwaysDiscardsLateVideoFrames(true) };

        // Create delegate.
        let delegate = FrameDelegate::new();
        let callback_holder = Arc::clone(&delegate.ivars().holder);

        // Configure session.
        unsafe {
            session.beginConfiguration();

            if session.canAddInput(&input) {
                session.addInput(&input);
            } else {
                session.commitConfiguration();
                return Err(CameraError::OpenFailed(
                    "Cannot add capture input to session".into(),
                ));
            }

            if session.canAddOutput(&output) {
                session.addOutput(&output);
            } else {
                session.commitConfiguration();
                return Err(CameraError::OpenFailed(
                    "Cannot add capture output to session".into(),
                ));
            }

            session.commitConfiguration();
        }

        // Determine frame dimensions from the device's active format.
        // We'll get the actual dimensions from the first frame, but provide
        // reasonable defaults based on the Lepton sensor.
        let (width, height) = Self::get_device_dimensions(&device);

        Ok(Self {
            device,
            session,
            output,
            delegate,
            callback_holder,
            capture_queue: None,
            format: captured_format,
            width,
            height,
            running: false,
        })
    }

    /// Start capturing frames, delivering each to `on_frame`.
    ///
    /// The callback is invoked on a dedicated serial dispatch queue.
    pub fn start<F>(&mut self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(CapturedFrame) + Send + 'static,
    {
        if self.running {
            return Ok(());
        }

        // Store the callback.
        {
            let mut guard = self.callback_holder.callback.lock();
            *guard = Some(Box::new(on_frame));
        }

        // Create a serial dispatch queue for frame delivery.
        let queue = DispatchQueue::new(
            "com.thermal.avfoundation.capture",
            dispatch2::DispatchQueueAttr::SERIAL,
        );

        // Set the delegate on the output.
        unsafe {
            self.output.setSampleBufferDelegate_queue(
                Some(ProtocolObject::from_ref(&*self.delegate)),
                Some(&queue),
            );
        }

        // Keep queue alive — AVFoundation needs it for the delegate callbacks.
        self.capture_queue = Some(queue);

        // Start the session on a separate thread to avoid blocking the Tauri command thread.
        eprintln!("[thermal-v2] AVFoundation: starting capture session...");
        let session_ptr = Retained::as_ptr(&self.session) as usize;
        let format = self.format;
        let w = self.width;
        let h = self.height;
        std::thread::spawn(move || {
            let session = session_ptr as *const AVCaptureSession;
            unsafe { (*session).startRunning() };
            eprintln!("[thermal-v2] AVFoundation: session started, format={:?}, {}x{}", format, w, h);
        });

        self.running = true;
        Ok(())
    }

    /// Stop capturing frames.
    pub fn stop(&mut self) {
        if !self.running {
            return;
        }

        unsafe { self.session.stopRunning() };

        // Remove the delegate to stop callbacks.
        unsafe {
            self.output.setSampleBufferDelegate_queue(None, None);
        }

        // Release the dispatch queue.
        self.capture_queue = None;

        // Clear the callback.
        {
            let mut guard = self.callback_holder.callback.lock();
            *guard = None;
        }

        self.running = false;
    }

    /// The pixel format being captured.
    pub fn format(&self) -> CapturedFormat {
        self.format
    }

    /// Frame width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Frame height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Whether the capture session is currently running.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// The underlying device name.
    #[allow(dead_code)]
    pub fn device_name(&self) -> String {
        unsafe { self.device.localizedName() }.to_string()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Inspect the output's available pixel formats and choose the best one.
    ///
    /// Prefers 16-bit gray (Y16 / kCVPixelFormatType_16Gray) for raw thermal
    /// data. Falls back to 32-bit BGRA which is universally supported.
    fn choose_pixel_format(output: &AVCaptureVideoDataOutput) -> (u32, CapturedFormat) {
        let available = unsafe { output.availableVideoCVPixelFormatTypes() };
        let count = available.count();

        // Check if 16-bit gray is available.
        for i in 0..count {
            let fmt_num = available.objectAtIndex(i);
            let fmt = fmt_num.unsignedIntValue();
            if fmt == kCVPixelFormatType_16Gray {
                return (kCVPixelFormatType_16Gray, CapturedFormat::Y16);
            }
        }

        // Fall back to BGRA.
        (kCVPixelFormatType_32BGRA, CapturedFormat::BGRA)
    }

    /// Configure the output's videoSettings to request the given pixel format.
    fn set_output_pixel_format(output: &AVCaptureVideoDataOutput, pixel_format: u32) {
        unsafe {
            // Build the settings dictionary using msg_send to avoid the
            // complex typed NSDictionary construction API.
            //
            // Equivalent ObjC:
            //   @{(NSString *)kCVPixelBufferPixelFormatTypeKey: @(pixel_format)}
            //
            // The kCVPixelBufferPixelFormatTypeKey string is "PixelFormatType".
            let key = NSString::from_str("PixelFormatType");
            let value = NSNumber::numberWithUnsignedInt(pixel_format);

            // Use +[NSDictionary dictionaryWithObjects:forKeys:count:]
            let mut keys_raw: [NonNull<AnyObject>; 1] =
                [NonNull::from(&*key).cast()];
            let mut vals_raw: [NonNull<AnyObject>; 1] =
                [NonNull::from(&*value).cast()];

            let dict: Retained<AnyObject> = msg_send![
                objc2::class!(NSDictionary),
                dictionaryWithObjects: vals_raw.as_mut_ptr(),
                forKeys: keys_raw.as_mut_ptr(),
                count: 1usize
            ];

            // Cast to the expected type and set on the output.
            // SAFETY: We constructed a valid NSDictionary<NSString, AnyObject>.
            let dict_ptr: *const AnyObject = &*dict;
            let dict_ref: &objc2_foundation::NSDictionary<NSString, AnyObject> =
                &*(dict_ptr as *const objc2_foundation::NSDictionary<NSString, AnyObject>);
            output.setVideoSettings(Some(dict_ref));
        }
    }

    /// Attempt to read the device's active format dimensions.
    ///
    /// Returns (width, height). Falls back to Lepton 3.x default (160x120)
    /// if the dimensions cannot be determined.
    fn get_device_dimensions(_device: &AVCaptureDevice) -> (usize, usize) {
        // The Lepton sensor is 160x120 (Lepton 3.x) or 80x60 (Lepton 2.x).
        // PureThermal may upscale to other resolutions depending on the
        // UVC descriptor. We'll update width/height from actual frame data
        // in the delegate callback when acquisition.rs consumes frames.
        //
        // For now, return the most common PureThermal resolution.
        (160, 120)
    }
}

impl Drop for AvCamera {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_format_debug() {
        assert_eq!(format!("{:?}", CapturedFormat::Y16), "Y16");
        assert_eq!(format!("{:?}", CapturedFormat::BGRA), "BGRA");
    }

    #[test]
    fn captured_format_equality() {
        assert_eq!(CapturedFormat::Y16, CapturedFormat::Y16);
        assert_eq!(CapturedFormat::BGRA, CapturedFormat::BGRA);
        assert_ne!(CapturedFormat::Y16, CapturedFormat::BGRA);
    }

    #[test]
    fn pixel_format_constants() {
        // kCVPixelFormatType_32BGRA = 'BGRA' = 0x42475241
        assert_eq!(kCVPixelFormatType_32BGRA, 0x42475241);
        // kCVPixelFormatType_16Gray = 'b16g' = 0x62313667
        assert_eq!(kCVPixelFormatType_16Gray, 0x62313667);
    }

    #[test]
    fn captured_frame_struct() {
        let frame = CapturedFrame {
            data: vec![0u8; 160 * 120 * 4],
            width: 160,
            height: 120,
            format: CapturedFormat::BGRA,
        };
        assert_eq!(frame.data.len(), 160 * 120 * 4);
        assert_eq!(frame.width, 160);
        assert_eq!(frame.height, 120);
        assert_eq!(frame.format, CapturedFormat::BGRA);
    }

    #[test]
    fn y16_frame_sizing() {
        let frame = CapturedFrame {
            data: vec![0u8; 160 * 120 * 2],
            width: 160,
            height: 120,
            format: CapturedFormat::Y16,
        };
        assert_eq!(frame.data.len(), 160 * 120 * 2);
    }
}
