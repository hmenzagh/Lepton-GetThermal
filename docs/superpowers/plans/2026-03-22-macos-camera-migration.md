# macOS Camera Migration: AVFoundation + nusb

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace libuvc (which causes kernel panics on modern macOS) with native macOS APIs: AVFoundation for video capture, nusb for USB control transfers to Lepton extension units.

**Architecture:** Two-layer replacement. Video frames are captured via AVFoundation (which cooperates with macOS's built-in UVC kernel driver instead of fighting it). Lepton SDK commands are sent via `nusb` (a pure Rust USB crate that uses IOKit natively on macOS). The processing pipeline (auto-gain, colorize) and entire frontend are unchanged.

**Tech Stack:** Tauri v2, Rust, `nusb` (USB control transfers via IOKit), `objc2` + `objc2-av-foundation` + `objc2-core-media` + `block2` (AVFoundation bridge), existing React frontend.

**Reference:** Current libuvc-based implementation in `src-tauri/src/uvc_ffi.rs`, `src-tauri/src/camera/acquisition.rs`, `src-tauri/src/camera/lepton.rs`.

**Why not libuvc:** libuvc uses libusb which bypasses macOS's USB stack. On modern macOS (Apple Silicon, macOS 15+), the kernel UVC driver (`com.apple.UVCService`) claims PureThermal devices. libuvc trying to also claim them causes `IOUSBHostFamily` kernel panics.

---

## File Structure (changes only)

```
src-tauri/
├── Cargo.toml                          # MODIFY: remove cmake, add nusb + objc2 crates
├── build.rs                            # MODIFY: remove libuvc cmake build
├── libuvc/                             # DELETE: remove git submodule
└── src/
    ├── uvc_ffi.rs                      # DELETE: no longer needed
    ├── usb_control.rs                  # CREATE: nusb-based USB control transfers
    ├── avfoundation.rs                 # CREATE: AVFoundation camera capture bridge
    ├── camera/
    │   ├── acquisition.rs             # REWRITE: use AVFoundation instead of libuvc
    │   ├── lepton.rs                  # MODIFY: use usb_control instead of uvc_ffi
    │   └── types.rs                   # MODIFY: minor adjustments
    └── commands/
        └── stream.rs                  # MINOR: adjust if acquisition API changes
```

---

## Task 1: Remove libuvc, Update Dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/build.rs`
- Delete: `src-tauri/src/uvc_ffi.rs`
- Remove: `src-tauri/libuvc/` (git submodule)

- [ ] **Step 1: Remove libuvc git submodule**

```bash
cd /Users/hmenzagh/misc/Thermal_V2
git submodule deinit -f src-tauri/libuvc
git rm -f src-tauri/libuvc
rm -rf .git/modules/src-tauri/libuvc
```

- [ ] **Step 2: Update Cargo.toml**

Remove `cmake` from build-dependencies. Add new dependencies:

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-shell = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
parking_lot = "0.12"
thiserror = "2"
nusb = "0.1"
objc2 = "0.6"
objc2-foundation = { version = "0.3", features = ["NSString", "NSArray", "NSError", "NSNotification"] }
objc2-av-foundation = { version = "0.3", features = ["AVCaptureDevice", "AVCaptureSession", "AVCaptureInput", "AVCaptureOutput", "AVCaptureVideoDataOutput", "AVMediaFormat"] }
objc2-core-media = { version = "0.3", features = ["CMSampleBuffer", "CMFormatDescription", "CMTime"] }
objc2-core-video = { version = "0.3", features = ["CVPixelBuffer", "CVBuffer", "CVImageBuffer"] }
block2 = "0.6"
dispatch2 = "0.2"

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

Note: exact version numbers may need adjustment. Run `cargo update` after editing to resolve.

- [ ] **Step 3: Simplify build.rs**

Replace `src-tauri/build.rs` with:

```rust
fn main() {
    tauri_build::build();
}
```

No more cmake, no more libuvc linking, no more libusb.

- [ ] **Step 4: Delete uvc_ffi.rs**

```bash
rm /Users/hmenzagh/misc/Thermal_V2/src-tauri/src/uvc_ffi.rs
```

- [ ] **Step 5: Comment out modules that depend on old code**

Temporarily comment out `mod uvc_ffi;` in `lib.rs` and any code in `acquisition.rs` and `lepton.rs` that references `uvc_ffi`. Replace module bodies with `// TODO: reimplemented in Task 2-4` stubs so `cargo check` passes.

Specifically in `lib.rs`:
```rust
// mod uvc_ffi;  // Removed: replaced by usb_control.rs and avfoundation.rs
mod usb_control;
mod avfoundation;
mod camera;
mod commands;
mod processing;
```

Create empty placeholder files:
- `src-tauri/src/usb_control.rs`: `// USB control transfers via nusb — Task 2`
- `src-tauri/src/avfoundation.rs`: `// AVFoundation camera capture — Task 4`

Stub out `acquisition.rs` and `lepton.rs` so they compile (empty structs, todo!() methods).

- [ ] **Step 6: Verify compilation**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo check 2>&1 | head -30
```

Expected: Compiles with warnings about unused/dead code. No errors.

- [ ] **Step 7: Verify existing processing tests still pass**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo test --lib processing 2>&1
```

Expected: 10 processing tests pass (palettes, autogain, colorize, pipeline). These are independent of the camera layer.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: remove libuvc, add nusb + AVFoundation dependencies"
```

---

## Task 2: USB Control Transfer Layer (nusb)

**Files:**
- Create: `src-tauri/src/usb_control.rs`

**Reference:** The Lepton SDK uses USB control transfers to read/write UVC extension units. The transfer format is:
- Read (GET_CUR): bmRequestType=0xA1, bRequest=0x81, wValue=(control_id << 8), wIndex=(unit_id << 8 | interface)
- Write (SET_CUR): bmRequestType=0x21, bRequest=0x01, wValue=(control_id << 8), wIndex=(unit_id << 8 | interface)

`nusb` provides native IOKit-based USB access on macOS that cooperates with kernel drivers.

- [ ] **Step 1: Write failing test for USB device discovery**

In `src-tauri/src/usb_control.rs`:

```rust
//! USB control transfer layer using nusb.
//! Provides UVC extension unit read/write for Lepton SDK commands.
//! Uses IOKit on macOS (via nusb) — cooperates with kernel UVC driver.

use crate::camera::types::CameraError;

/// PureThermal USB identifiers
const PT_VID: u16 = 0x1e4e;
const PT_PID: u16 = 0x0100;

/// UVC class-specific request codes
const UVC_SET_CUR: u8 = 0x01;
const UVC_GET_CUR: u8 = 0x81;

/// UVC request type constants
const USB_TYPE_CLASS: u8 = 0x01 << 5;
const USB_RECIP_INTERFACE: u8 = 0x01;
const USB_DIR_OUT: u8 = 0x00;
const USB_DIR_IN: u8 = 0x80;

/// UVC interface number for extension units (typically interface 0 for video control)
const UVC_VC_INTERFACE: u16 = 0;

/// Manages USB connection to PureThermal device for control transfers.
pub struct UsbControl {
    device: nusb::Device,
    interface: nusb::Interface,
}

impl UsbControl {
    /// Find and open the PureThermal device.
    pub fn connect() -> Result<Self, CameraError> {
        todo!()
    }

    /// Read from a UVC extension unit (GET_CUR).
    pub fn get_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<usize, CameraError> {
        todo!()
    }

    /// Write to a UVC extension unit (SET_CUR).
    pub fn set_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<(), CameraError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_type_constants_are_correct() {
        // GET_CUR: device-to-host, class, interface
        let get_req_type = USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE;
        assert_eq!(get_req_type, 0xA1);

        // SET_CUR: host-to-device, class, interface
        let set_req_type = USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE;
        assert_eq!(set_req_type, 0x21);
    }

    #[test]
    fn windex_encoding() {
        // wIndex = (unit_id << 8) | interface_number
        let unit_id: u8 = 3; // AGC
        let interface: u16 = 0;
        let windex = (unit_id as u16) << 8 | interface;
        assert_eq!(windex, 0x0300);
    }

    #[test]
    fn wvalue_encoding() {
        // wValue = (control_id << 8)
        let control_id: u8 = 1;
        let wvalue = (control_id as u16) << 8;
        assert_eq!(wvalue, 0x0100);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass (pure logic tests)**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo test --lib usb_control -- --nocapture
```

Expected: 3 tests pass. The `todo!()` methods are not called by these tests.

- [ ] **Step 3: Implement connect()**

```rust
impl UsbControl {
    pub fn connect() -> Result<Self, CameraError> {
        let device_info = nusb::list_devices()
            .map_err(|e| CameraError::UvcError(format!("Failed to list USB devices: {e}")))?
            .find(|d| d.vendor_id() == PT_VID && d.product_id() == PT_PID)
            .ok_or(CameraError::DeviceNotFound)?;

        let device = device_info
            .open()
            .map_err(|e| CameraError::OpenFailed(format!("Failed to open device: {e}")))?;

        // Claim the video control interface (interface 0) for extension unit access.
        // nusb uses IOKit on macOS which cooperates with the kernel UVC driver.
        let interface = device
            .claim_interface(UVC_VC_INTERFACE as u8)
            .map_err(|e| CameraError::OpenFailed(format!("Failed to claim interface: {e}")))?;

        Ok(Self { device, interface })
    }
}
```

**Important note:** If `claim_interface` fails because the kernel driver holds it, we may need to skip claiming and send control transfers at the device level instead. The plan includes a fallback in Step 5.

- [ ] **Step 4: Implement get_ctrl() and set_ctrl()**

```rust
impl UsbControl {
    pub fn get_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<usize, CameraError> {
        let request_type = USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE;
        let wvalue = (control_id as u16) << 8;
        let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;

        let result = self.interface.control_in(nusb::transfer::ControlIn {
            control_type: nusb::transfer::ControlType::Class,
            recipient: nusb::transfer::Recipient::Interface,
            request: UVC_GET_CUR,
            value: wvalue,
            index: windex,
            length: data.len() as u16,
        })
        .into_result()
        .map_err(|e| CameraError::LeptonError(format!(
            "get_ctrl failed (unit={unit_id}, ctrl={control_id}): {e}"
        )))?;

        let len = result.len().min(data.len());
        data[..len].copy_from_slice(&result[..len]);
        Ok(len)
    }

    pub fn set_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<(), CameraError> {
        let wvalue = (control_id as u16) << 8;
        let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;

        self.interface.control_out(nusb::transfer::ControlOut {
            control_type: nusb::transfer::ControlType::Class,
            recipient: nusb::transfer::Recipient::Interface,
            request: UVC_SET_CUR,
            value: wvalue,
            index: windex,
            data,
        })
        .into_result()
        .map_err(|e| CameraError::LeptonError(format!(
            "set_ctrl failed (unit={unit_id}, ctrl={control_id}): {e}"
        )))?;

        Ok(())
    }
}
```

**Note:** The `nusb` API may differ slightly from what's shown here. The implementer MUST read the actual `nusb` docs/examples (`cargo doc -p nusb --open`) and adjust the control transfer API calls accordingly. The key parameters (request type, wValue, wIndex encoding) are correct — only the Rust API surface may differ.

- [ ] **Step 5: Fallback — if claim_interface fails**

If macOS refuses to let us claim interface 0 (because the kernel UVC driver holds it), try sending control transfers at the device level instead:

```rust
// Alternative: use device-level control transfers if interface claim fails
pub fn connect() -> Result<Self, CameraError> {
    // ... device discovery same as above ...

    // Try claiming interface; if it fails, proceed without claim.
    // On macOS, USB control transfers to endpoint 0 may work even
    // without explicitly claiming the target interface.
    let interface = match device.claim_interface(UVC_VC_INTERFACE as u8) {
        Ok(iface) => iface,
        Err(e) => {
            eprintln!("[thermal-v2] Could not claim interface 0: {e}. Attempting device-level control transfers.");
            // nusb may allow control transfers on an unclaimed interface
            // or we need to use device.control_in/control_out directly
            return Err(CameraError::OpenFailed(format!("Cannot claim USB interface: {e}")));
        }
    };
    // ...
}
```

The implementer should test on the actual hardware and adjust. If `claim_interface` fails, investigate `nusb`'s device-level control transfer API or use `detach_kernel_driver()` if available.

- [ ] **Step 6: Verify compilation**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo check 2>&1 | head -20
```

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/usb_control.rs
git commit -m "feat: add nusb-based USB control transfer layer for Lepton commands"
```

---

## Task 3: Migrate lepton.rs to New USB Layer

**Files:**
- Modify: `src-tauri/src/camera/lepton.rs`
- Modify: `src-tauri/src/camera/types.rs` (if needed)

The command ID mapping logic and all high-level API methods stay the same. Only the low-level `get_attribute` / `set_attribute` methods change to use `UsbControl` instead of `uvc_get_ctrl` / `uvc_set_ctrl`.

- [ ] **Step 1: Update LeptonController to use UsbControl**

Replace the `devh` raw pointer + Mutex pattern with a reference to `UsbControl`:

```rust
use std::sync::Arc;
use crate::usb_control::UsbControl;
use super::types::CameraError;

pub struct LeptonController {
    usb: Arc<UsbControl>,
    lock: parking_lot::Mutex<()>,
}

unsafe impl Send for LeptonController {}
unsafe impl Sync for LeptonController {}

impl LeptonController {
    pub fn new(usb: Arc<UsbControl>) -> Self {
        Self {
            usb,
            lock: parking_lot::Mutex::new(()),
        }
    }

    pub fn get_attribute(
        &self,
        command_id: u16,
        word_length: usize,
    ) -> Result<Vec<u16>, CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);
        let byte_length = word_length * 2;

        let mut buf = vec![0u8; byte_length];
        self.usb.get_ctrl(unit_id, control_id, &mut buf)?;

        let words: Vec<u16> = buf
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(words)
    }

    pub fn set_attribute(
        &self,
        command_id: u16,
        data: &[u16],
    ) -> Result<(), CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);

        let mut buf: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.usb.set_ctrl(unit_id, control_id, &mut buf)?;
        Ok(())
    }

    // All other methods (get_u16, set_u16, run_command, get_agc_enable, etc.)
    // remain EXACTLY the same — they all delegate to get_attribute/set_attribute.
}
```

- [ ] **Step 2: Keep all command ID constants and mapping functions unchanged**

The following must remain exactly as-is (they are pure logic with no FFI dependency):
- `command_to_unit_id()`
- `command_to_control_id()`
- All `LEP_*` command ID constants
- All high-level API methods (`get_agc_enable`, `set_agc_enable`, `perform_ffc`, etc.)

- [ ] **Step 3: Verify existing Lepton command mapping tests pass**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo test --lib camera::lepton -- --nocapture
```

Expected: All command mapping tests pass (they test pure logic, not USB I/O).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/camera/lepton.rs
git commit -m "refactor: migrate LeptonController from libuvc to nusb USB control layer"
```

---

## Task 4: AVFoundation Video Capture

**Files:**
- Create: `src-tauri/src/avfoundation.rs`
- Rewrite: `src-tauri/src/camera/acquisition.rs`

This is the most complex task. We use AVFoundation to discover the PureThermal camera (which macOS sees as a standard UVC device), start a capture session, and receive frames.

**Key insight:** macOS's built-in UVC driver handles all USB streaming. AVFoundation gives us frames in a safe, kernel-cooperating way. No kernel panics.

**Format strategy:**
- Try to capture in Y16 format first (CoreVideo `kCVPixelFormatType_16Gray` = `0x00000010` or `kCVPixelFormatType_OneComponent16Half`)
- If Y16 is not available, capture in `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` (NV12) or BGRA and note that the PureThermal's built-in AGC will be used instead of our software AGC
- The processing pipeline adapts based on what format we get

- [ ] **Step 1: Create AVFoundation bridge**

In `src-tauri/src/avfoundation.rs`, create the Objective-C bridge:

```rust
//! AVFoundation camera capture bridge.
//! Discovers PureThermal as a UVC camera and captures frames via macOS's native UVC driver.

use std::sync::Arc;
use parking_lot::Mutex;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSString, NSArray, NSObjectProtocol};
use objc2_av_foundation::*;
use objc2_core_media::*;
use objc2_core_video::*;
use block2::RcBlock;

use crate::camera::types::CameraError;

/// Pixel format we receive from AVFoundation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapturedFormat {
    /// Raw 16-bit grayscale (ideal — our processing pipeline handles this)
    Y16,
    /// 32-bit BGRA (camera's built-in AGC applied — we display directly)
    BGRA,
}

/// Frame data received from AVFoundation
pub struct CapturedFrame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub format: CapturedFormat,
}

/// AVFoundation-based camera capture.
pub struct AvCamera {
    session: Retained<AVCaptureSession>,
    format: CapturedFormat,
    frame_width: usize,
    frame_height: usize,
}

impl AvCamera {
    /// Discover PureThermal camera and create capture session.
    /// Returns None if no PureThermal device found.
    pub fn discover() -> Result<Self, CameraError> {
        todo!("Implement AVFoundation device discovery")
    }

    /// Start capturing frames. Calls `on_frame` for each frame.
    pub fn start<F>(&self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(CapturedFrame) + Send + 'static,
    {
        todo!("Implement AVFoundation capture start")
    }

    /// Stop capture session.
    pub fn stop(&self) {
        todo!("Implement AVFoundation capture stop")
    }

    pub fn format(&self) -> CapturedFormat {
        self.format
    }

    pub fn width(&self) -> usize {
        self.frame_width
    }

    pub fn height(&self) -> usize {
        self.frame_height
    }
}
```

- [ ] **Step 2: Implement device discovery**

The PureThermal device appears as an external UVC camera. Use AVFoundation to find it:

```rust
impl AvCamera {
    pub fn discover() -> Result<Self, CameraError> {
        // Discover external video capture devices
        let device_type = unsafe { AVCaptureDeviceTypeExternal };
        let media_type = unsafe { AVMediaTypeVideo };

        let discovery = unsafe {
            AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
                &NSArray::from_retained_slice(&[device_type.copy()]),
                Some(media_type),
                AVCaptureDevicePosition::Unspecified,
            )
        };

        let devices = unsafe { discovery.devices() };

        // Find PureThermal device by name
        let pt_device = devices
            .iter()
            .find(|d| {
                let name = unsafe { d.localizedName() };
                let name_str = name.to_string();
                name_str.contains("PureThermal") || name_str.contains("Lepton")
            })
            .ok_or(CameraError::DeviceNotFound)?;

        // Check available formats for Y16 support
        let formats = unsafe { pt_device.formats() };
        let mut selected_format = CapturedFormat::BGRA;

        for fmt in formats.iter() {
            let desc = unsafe { fmt.formatDescription() };
            let pixel_format = unsafe {
                CMFormatDescriptionGetMediaSubType(desc.as_ref())
            };
            // kCVPixelFormatType_16Gray = 0x00000010 (16)
            if pixel_format == 16 {
                selected_format = CapturedFormat::Y16;
                break;
            }
        }

        // Create and configure capture session
        let session = unsafe { AVCaptureSession::new() };

        let input = unsafe {
            AVCaptureDeviceInput::deviceInputWithDevice_error(pt_device)
        }.map_err(|e| CameraError::OpenFailed(format!("Cannot create input: {e}")))?;

        unsafe {
            session.beginConfiguration();
            if session.canAddInput(&input) {
                session.addInput(&input);
            }
            session.commitConfiguration();
        }

        // Determine dimensions from the device's active format
        let (width, height) = (160usize, 120usize); // Lepton default; will be refined from format descriptors

        Ok(Self {
            session,
            format: selected_format,
            frame_width: width,
            frame_height: height,
        })
    }
}
```

**Note:** The exact `objc2-av-foundation` API may differ. The implementer MUST consult the crate docs and Apple's AVFoundation documentation. The logic (discover devices → find PureThermal → check pixel formats → create session) is correct; the Rust bindings syntax may need adjustment.

- [ ] **Step 3: Implement frame capture with callback**

This uses `AVCaptureVideoDataOutput` with a delegate callback:

```rust
impl AvCamera {
    pub fn start<F>(&mut self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(CapturedFrame) + Send + 'static,
    {
        let output = unsafe { AVCaptureVideoDataOutput::new() };

        // Configure desired pixel format
        let pixel_format = match self.format {
            CapturedFormat::Y16 => 16u32,           // kCVPixelFormatType_16Gray
            CapturedFormat::BGRA => 0x42475241u32,  // kCVPixelFormatType_32BGRA = 'BGRA'
        };

        // Set up the output delegate with a dispatch queue
        // Note: The actual delegate pattern in objc2 requires creating an
        // Objective-C class that conforms to AVCaptureVideoDataOutputSampleBufferDelegate.
        // This is complex in Rust — see objc2 documentation for declare_class! macro.

        // The callback extracts pixel data from CMSampleBuffer:
        // 1. Get CVPixelBuffer from CMSampleBuffer
        // 2. Lock base address
        // 3. Copy pixel data
        // 4. Unlock base address
        // 5. Call on_frame with CapturedFrame

        unsafe {
            self.session.beginConfiguration();
            if self.session.canAddOutput(&output) {
                self.session.addOutput(&output);
            }
            self.session.commitConfiguration();
            self.session.startRunning();
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        unsafe {
            self.session.stopRunning();
        }
    }
}
```

**Important:** The delegate pattern is the hardest part. The implementer needs to:
1. Use `objc2::declare_class!` to create a Rust class that implements `AVCaptureVideoDataOutputSampleBufferDelegate`
2. In the `captureOutput:didOutputSampleBuffer:fromConnection:` method, extract the pixel buffer and invoke the Rust callback
3. Use a `dispatch2` queue for the delegate's callback queue

This is well-documented in the `objc2` crate examples but is non-trivial. The implementer should study:
- `objc2` declare_class! macro
- Apple's AVCaptureVideoDataOutputSampleBufferDelegate protocol
- CVPixelBuffer data extraction pattern

- [ ] **Step 4: Verify compilation**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo check 2>&1 | head -30
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/avfoundation.rs
git commit -m "feat: add AVFoundation camera capture bridge"
```

---

## Task 5: Rewrite Camera Acquisition

**Files:**
- Rewrite: `src-tauri/src/camera/acquisition.rs`
- Modify: `src-tauri/src/commands/stream.rs` (if API changes)
- Modify: `src-tauri/src/lib.rs` (update state management)

Replace the libuvc-based `CameraAcquisition` with one that uses `AvCamera` for video and `UsbControl` (via `LeptonController`) for commands.

- [ ] **Step 1: Rewrite CameraAcquisition**

```rust
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use parking_lot::Mutex;

use crate::avfoundation::{AvCamera, CapturedFormat, CapturedFrame};
use crate::processing::{self, palettes::Palette, FrameResult};
use super::types::*;

pub struct CameraAcquisition {
    camera: AvCamera,
    streaming: Arc<AtomicBool>,
    current_palette: Arc<Mutex<Palette>>,
}

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

        self.camera.start(move |captured: CapturedFrame| {
            let current_palette = *palette.lock();

            let result = match camera_format {
                CapturedFormat::Y16 => {
                    // Use our processing pipeline: Y16 → auto-gain → colorize → RGBA
                    processing::process_frame(&captured.data, w, h, current_palette)
                }
                CapturedFormat::BGRA => {
                    // Camera applied its own AGC — convert BGRA to RGBA directly
                    let rgba = bgra_to_rgba(&captured.data);
                    FrameResult {
                        rgba,
                        width: w,
                        height: h,
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
            rgba.push(chunk[2]); // R (was at index 2 in BGRA)
            rgba.push(chunk[1]); // G
            rgba.push(chunk[0]); // B (was at index 0 in BGRA)
            rgba.push(chunk[3]); // A
        }
    }
    rgba
}
```

- [ ] **Step 2: Update stream.rs commands**

The `start_stream` command no longer takes width/height/fps since AVFoundation handles format negotiation. Update the Tauri command signature:

```rust
#[tauri::command]
pub fn start_stream(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut cam_guard = state.camera.lock();
    let cam = cam_guard.as_mut().ok_or("Camera not connected")?;

    cam.start_stream(move |frame_result| {
        let event = FrameEvent {
            data: BASE64.encode(&frame_result.rgba),
            width: frame_result.width,
            height: frame_result.height,
            min_val: frame_result.stats.min_val,
            max_val: frame_result.stats.max_val,
        };
        let _ = app.emit("thermal-frame", event);
    })
    .map_err(|e| e.to_string())
}
```

Also update `connect_camera` to create `UsbControl` and `LeptonController`:

```rust
#[tauri::command]
pub fn connect_camera(state: State<'_, AppState>) -> Result<String, String> {
    eprintln!("[thermal-v2] Connecting via AVFoundation + nusb...");

    // Video capture via AVFoundation
    let cam = CameraAcquisition::connect().map_err(|e| {
        eprintln!("[thermal-v2] AVFoundation connection failed: {e}");
        e.to_string()
    })?;
    eprintln!("[thermal-v2] AVFoundation camera discovered");

    // USB control via nusb for Lepton commands
    let usb = match crate::usb_control::UsbControl::connect() {
        Ok(usb) => {
            eprintln!("[thermal-v2] USB control connected");
            Some(std::sync::Arc::new(usb))
        }
        Err(e) => {
            eprintln!("[thermal-v2] USB control failed (Lepton commands unavailable): {e}");
            None
        }
    };

    let lepton = usb.map(|u| std::sync::Arc::new(crate::camera::lepton::LeptonController::new(u)));

    *state.camera.lock() = Some(cam);
    *state.lepton.lock() = lepton.clone();

    let part = lepton
        .as_ref()
        .and_then(|l| l.get_part_number().ok())
        .unwrap_or_default();
    Ok(part)
}
```

- [ ] **Step 3: Update lib.rs AppState**

Update the AppState to match the new types:

```rust
pub struct AppState {
    pub camera: Mutex<Option<CameraAcquisition>>,
    pub lepton: Mutex<Option<Arc<LeptonController>>>,
}
```

This is the same structure — just ensure imports point to the new `CameraAcquisition`.

- [ ] **Step 4: Update frontend startStream call**

In `src/hooks/useCamera.ts`, remove the width/height/fps parameters since AVFoundation handles format negotiation:

```typescript
const startStream = useCallback(async () => {
    try {
        await invoke("start_stream");
        setState("streaming");
    } catch (e) {
        setError(String(e));
    }
}, []);
```

And in `src/App.tsx`:
```typescript
const handleConnect = useCallback(async () => {
    await camera.connect();
    await camera.startStream();
}, [camera.connect, camera.startStream]);
```

- [ ] **Step 5: Verify compilation**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo check 2>&1 | head -30
```

- [ ] **Step 6: Verify processing tests still pass**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo test --lib processing 2>&1
```

Expected: All 10 processing tests pass (unchanged).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: replace libuvc acquisition with AVFoundation + nusb architecture"
```

---

## Task 6: Hardware Testing & Polish

**Files:** No new files — testing and fixes only.

- [ ] **Step 1: Run all Rust tests**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo test --lib -- --nocapture
```

Expected: All processing + lepton command mapping tests pass.

- [ ] **Step 2: Run TypeScript checks**

```bash
cd /Users/hmenzagh/misc/Thermal_V2
npx tsc --noEmit
```

- [ ] **Step 3: Run clippy**

```bash
cd /Users/hmenzagh/misc/Thermal_V2/src-tauri
cargo clippy -- -W clippy::all 2>&1 | head -30
```

Fix any warnings.

- [ ] **Step 4: Test with PureThermal hardware**

```bash
cd /Users/hmenzagh/misc/Thermal_V2
npm run tauri dev
```

Verify:
- App opens without kernel panic
- "Connect Camera" button works
- PureThermal device is discovered via AVFoundation
- Video frames are displayed on canvas
- Lepton commands work (FFC, AGC toggle, palette change)
- Temperature display works (if radiometry supported)

- [ ] **Step 5: Debug and fix issues**

Common issues to expect:
1. **AVFoundation pixel format mismatch** — Check what format PureThermal actually reports. May need to adjust format negotiation.
2. **nusb interface claim failure** — macOS may block USB interface claim. Try without claim_interface and use device-level control transfers.
3. **Frame dimensions** — Verify 160x120 matches what AVFoundation reports for the device.
4. **Delegate callback issues** — The objc2 delegate pattern is tricky. Watch for memory management and thread safety.

- [ ] **Step 6: Commit final state**

```bash
git add -A
git commit -m "chore: hardware testing and polish for AVFoundation migration"
```

---

## Notes

### Why Two Separate Libraries

- **AVFoundation (video):** Cooperates with macOS kernel UVC driver. The kernel handles USB isochronous transfers, format negotiation, and frame assembly. No conflict.
- **nusb (Lepton commands):** Sends USB control transfers via IOKit. Control transfers go through endpoint 0 and don't require claiming the video streaming interface.

### Fallback Strategy

If AVFoundation cannot capture Y16 from PureThermal:
1. **First try:** Capture BGRA — the PureThermal board applies its own AGC and pseudo-color. We display directly.
2. **If that fails:** Put PureThermal in Y16 mode via Lepton command (`LEP_VID_OUTPUT_FORMAT`), then try AVFoundation again.
3. **Last resort:** Use `nusb` for isochronous USB transfers (complex but avoids kernel conflict since nusb uses IOKit natively).

### Complexity Assessment

| Task | Risk | Effort |
|------|------|--------|
| Task 1: Remove libuvc | Low | 30 min |
| Task 2: USB control layer | Low-Medium | 2-4 hours |
| Task 3: Migrate lepton.rs | Low | 1 hour |
| Task 4: AVFoundation capture | **High** | 1-2 days |
| Task 5: Integration | Medium | 2-4 hours |
| Task 6: Hardware testing | Medium | 1-2 hours |

Task 4 is the hardest — the `objc2` delegate pattern for `AVCaptureVideoDataOutputSampleBufferDelegate` is non-trivial in Rust. Budget extra time there.

### Important Implementation Notes (from plan review)

1. **`nusb` is async-first.** Its `control_in`/`control_out` return futures. Use `futures_lite::future::block_on()` or `pollster::block_on()` to call from synchronous code. Add `futures-lite` or `pollster` to dependencies.

2. **AVFoundation frameworks linking.** The `objc2-*` crates may handle framework linking via `#[link]` attributes automatically. If not, add to `build.rs`:
   ```rust
   println!("cargo:rustc-link-lib=framework=AVFoundation");
   println!("cargo:rustc-link-lib=framework=CoreMedia");
   println!("cargo:rustc-link-lib=framework=CoreVideo");
   ```

3. **Frame dimensions.** Extract actual dimensions from `CMVideoFormatDescriptionGetDimensions()` during device discovery instead of hardcoding 160x120. Lepton 3.x = 160x120, Lepton 2.x = 80x60.

4. **USB control without interface claim.** If macOS blocks `claim_interface(0)`, `nusb` may still allow control transfers at the device level. Test on hardware and adjust.

5. **Graceful degradation.** If `UsbControl::connect()` fails, the camera still streams via AVFoundation — only Lepton commands (AGC, FFC, etc.) are unavailable. The frontend should handle this gracefully (controls grayed out).

### objc2 Resources

- `objc2` crate docs: cargo doc -p objc2 --open
- `objc2-av-foundation` examples (if any)
- Apple AVFoundation Programming Guide (Objective-C, translate patterns to objc2)
- The `declare_class!` macro documentation in objc2 for creating delegate classes
