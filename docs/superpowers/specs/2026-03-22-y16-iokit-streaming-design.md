# Y16 IOKit Streaming — Design Spec

**Date:** 2026-03-22
**Status:** Draft
**Objective:** Replace AVFoundation video capture with direct IOKit isochronous USB transfers to get raw Y16 (16-bit) thermal data from the PureThermal device.

## Context

The PureThermal USB device wraps a FLIR Lepton 3.x thermal sensor. Currently, the app uses macOS AVFoundation for video capture, which only exposes BGRA pixel format. In BGRA mode, the PureThermal applies its own internal AGC and colorization — our software palettes, polarity, and AGC controls have no effect.

The Lepton outputs raw 16-bit radiometric data. The PureThermal exposes this as Y16 (16-bit grayscale) via UVC, but AVFoundation does not surface this format. By switching to direct IOKit USB isochronous transfers, we can read Y16 frames and regain full control over the image processing pipeline.

## Constraints

- **macOS only** — IOKit is the sole USB access method
- **Exclusive USB access** — `USBDeviceOpen` required; AVFoundation cannot coexist
- **Frame rate** — Accept all frames from PureThermal (up to ~27 FPS with duplicates; ~9 FPS unique from Lepton)
- **Resolution** — 160×120 (Lepton 3.x native)
- **Cleanup** — Remove all AVFoundation code and related dependencies

## Architecture

```
PureThermal USB Device (Lepton 3.x sensor)
    │
    ├─ [Video Endpoint - Isochronous]
    │       ↓
    │   IOKit: USBDeviceOpen (exclusive access)
    │       ↓
    │   SetConfiguration
    │       ↓
    │   CreateInterfaceIterator → find VideoStreaming interface
    │       ↓
    │   USBInterfaceOpen (on VideoStreaming interface)
    │       ↓
    │   UVC descriptor parsing → find Y16 format + 160×120 frame
    │       ↓
    │   UVC Probe/Commit (class-specific control on VS interface)
    │       ↓
    │   SetAlternateInterface (activate isoch bandwidth)
    │       ↓
    │   CreateInterfaceAsyncEventSource → CFRunLoop thread
    │       ↓
    │   ReadIsochPipeAsync (continuous isochronous reads with IOUSBIsocFrame arrays)
    │       ↓
    │   UVC payload parsing → frame reassembly (FID bit toggle)
    │       ↓
    │   Complete Y16 frame (160×120×2 = 38,400 bytes, little-endian)
    │       ↓
    │   auto_gain() → colorize() → RGBA
    │       ↓
    │   Tauri emit("thermal-frame") → React canvas
    │
    └─ [Control Endpoint - Extension Units]
            ↓
        Same device handle, ControlRequest on appropriate interface
            ↓
        LeptonController: AGC, polarity, FFC, spotmeter, radiometry
```

### Ownership Graph

```
AppState (Tauri managed state)
  └── UsbStream (owns device handle + interface handles)
       ├── provides start_stream() / stop_stream() for video
       ├── provides control_transfer() for Lepton commands
       │
       ├── CameraAcquisition uses UsbStream for streaming + processing
       └── LeptonController uses UsbStream for SDK commands
```

`connect_camera` creates a single `UsbStream`, then both `CameraAcquisition` and `LeptonController` derive from it. This replaces the current pattern where `CameraAcquisition` and `UsbControl` are created independently.

## Components

### 1. `usb_stream.rs` — New IOKit USB Streaming Module

Replaces `avfoundation.rs`. Written in Rust with FFI calls to an extended `usb_helper.c`.

**Public API:**

```rust
pub struct UsbStream {
    // Internal: device handle, interface handle, streaming thread, run loop
    // Requires explicit Send/Sync via unsafe impl (IOKit handles are raw pointers)
}

// SAFETY: IOKit handles are used from a single run-loop thread for streaming
// and synchronized via Mutex for control transfers.
unsafe impl Send for UsbStream {}
unsafe impl Sync for UsbStream {}

impl UsbStream {
    /// Discover and open the PureThermal device (VID 0x1e4e, PID 0x0100)
    pub fn open() -> Result<Self, CameraError>;

    /// Start Y16 streaming with a frame callback
    pub fn start_stream<F>(&self, callback: F) -> Result<(), CameraError>
    where F: Fn(&[u8], u32, u32) + Send + 'static;
    // callback args: raw Y16 bytes (LE), width, height

    /// Stop streaming, release alt-setting
    pub fn stop_stream(&self) -> Result<(), CameraError>;

    /// Send control transfer (for Lepton commands via UVC extension units)
    pub fn control_transfer(&self, request: ControlRequest) -> Result<Vec<u8>, CameraError>;

    /// Close device, release all resources
    pub fn close(self);
}
```

Note: `start_stream` callback receives `&[u8]` (raw little-endian Y16 bytes) to match the existing `process_frame` signature in `acquisition.rs`, which already handles `u8` → `u16` conversion via `u16::from_le_bytes`.

**Internal responsibilities:**

#### 1a. Device Discovery & Open
- Iterate IOKit USB devices matching VID/PID `0x1e4e:0x0100`
- Call `USBDeviceOpen` for exclusive access
- Call `SetConfiguration` (required before interface access)
- Store device handle

#### 1b. Interface Acquisition
- Use `CreateInterfaceIterator` (via `IOUSBFindInterfaceRequest`) to find the VideoStreaming interface (UVC class `0x0E`, subclass `0x02`)
- Call `IOCreatePlugInInterfaceForService` + `QueryInterface` to get `IOUSBInterfaceInterface` handle
- Call `USBInterfaceOpen` on the interface (required before `SetAlternateInterface` or pipe operations)
- Also find and open the VideoControl interface (class `0x0E`, subclass `0x01`) for extension unit control transfers

#### 1c. UVC Descriptor Parsing
- Read configuration descriptor via `GetConfigurationDescriptorPtr`
- Parse UVC VideoStreaming interface descriptors:
  - `VS_FORMAT_UNCOMPRESSED` — find format with Y16 GUID (`59553136-0000-1000-8000-00aa00389b71`)
  - `VS_FRAME_UNCOMPRESSED` — find 160×120 frame descriptor, extract min frame interval
  - Note the format index and frame index for probe/commit
- Find the alt-setting with isochronous endpoint and sufficient `wMaxPacketSize`

#### 1d. UVC Probe/Commit Negotiation
- Probe/Commit is a **class-specific control transfer on the VideoStreaming interface** (not device endpoint 0)
- Uses `ControlRequest` on the VS interface handle with:
  - `bmRequestType`: Host-to-Device, Class, Interface
  - `wIndex`: VideoStreaming interface number (typically 1)
- Send `VS_PROBE_CONTROL SET_CUR` with desired format index, frame index, frame interval
- Read back `VS_PROBE_CONTROL GET_CUR` to confirm negotiated parameters
- Send `VS_COMMIT_CONTROL SET_CUR` to lock parameters
- Structure: UVC Video Probe Control (26 or 34 bytes depending on UVC version)

#### 1e. Stream Activation
- `SetAlternateInterface` to the alt-setting with isochronous bandwidth
- This activates the isochronous endpoint for data transfer

#### 1f. Isochronous Transfer Loop (CFRunLoop Thread)

The IOKit async API requires a CFRunLoop for completion callbacks:

1. Call `CreateInterfaceAsyncEventSource` on the interface to get a `CFRunLoopSourceRef`
2. Spawn a dedicated thread that:
   - Adds the event source to its `CFRunLoop` via `CFRunLoopAddSource`
   - Submits initial `ReadIsochPipeAsync` requests (double-buffered)
   - Calls `CFRunLoopRun()` — blocks until stopped
3. Each `ReadIsochPipeAsync` call requires:
   - A buffer for the payload data
   - An `IOUSBIsocFrame` array specifying expected size per microframe
   - A frame number (use `GetBusFrameNumber` + offset, or 0 for "next available")
   - For USB 2.0 high-speed: each microframe is 125µs
   - Recommended: 32-64 frames per transfer request
4. On completion callback:
   - Parse UVC payload headers from each microframe's data
   - Accumulate payload data into frame buffer
   - On FID toggle or EOF: deliver completed frame, start new buffer
   - If ERR bit (bit 6) is set: discard current frame
   - Drop frames with incorrect size (≠ 38,400 bytes)
   - Resubmit the transfer request (double-buffer swap)
5. To stop: `CFRunLoopStop` from another thread, then join

#### 1g. Cleanup
- Stop CFRunLoop thread
- Cancel pending isochronous transfers
- `SetAlternateInterface(0)` to release bandwidth
- `USBInterfaceClose` on interface handles
- `USBDeviceClose` on device handle

### 2. Extended `usb_helper.c` — IOKit C Functions

New C functions exposed via FFI. **Replaces the global-state pattern** (`static IOUSBDeviceInterface **g_device`) with explicit handle passing — all functions take handles as parameters.

```c
// Device lifecycle
IOReturn thermal_usb_open_exclusive(io_service_t device, IOUSBDeviceInterface ***dev_out);
IOReturn thermal_usb_set_configuration(IOUSBDeviceInterface **dev, UInt8 config);
IOReturn thermal_usb_close_exclusive(IOUSBDeviceInterface **dev);

// Interface lifecycle
IOReturn thermal_usb_find_interface(IOUSBDeviceInterface **dev,
    UInt8 iface_class, UInt8 iface_subclass,
    IOUSBInterfaceInterface ***intf_out);
IOReturn thermal_usb_open_interface(IOUSBInterfaceInterface **intf);
IOReturn thermal_usb_close_interface(IOUSBInterfaceInterface **intf);
IOReturn thermal_usb_set_alt_interface(IOUSBInterfaceInterface **intf,
    UInt8 alt_setting);

// Descriptor access
IOReturn thermal_usb_get_config_descriptor(IOUSBDeviceInterface **dev,
    UInt8 *buf, UInt16 *len);

// UVC probe/commit (class-specific control on VS interface)
IOReturn thermal_usb_probe_set(IOUSBInterfaceInterface **intf,
    UInt8 vs_iface_num, void *probe_data, UInt16 len);
IOReturn thermal_usb_probe_get(IOUSBInterfaceInterface **intf,
    UInt8 vs_iface_num, void *probe_data, UInt16 len);
IOReturn thermal_usb_commit(IOUSBInterfaceInterface **intf,
    UInt8 vs_iface_num, void *probe_data, UInt16 len);

// Isochronous streaming
IOReturn thermal_usb_create_async_event_source(IOUSBInterfaceInterface **intf,
    CFRunLoopSourceRef *source_out);
IOReturn thermal_usb_read_isoch_async(IOUSBInterfaceInterface **intf,
    UInt8 pipe_ref, void *buf, UInt64 frame_start,
    UInt32 num_frames, IOUSBIsocFrame *frame_list,
    IOAsyncCallback1 callback, void *context);
IOReturn thermal_usb_get_bus_frame_number(IOUSBInterfaceInterface **intf,
    UInt64 *frame_number, AbsoluteTime *at_time);

// Control transfers (replaces existing non-exclusive version)
// wIndex includes interface number in low byte for extension unit requests
IOReturn thermal_usb_ctrl_transfer(IOUSBInterfaceInterface **intf,
    UInt8 request, UInt16 value, UInt16 index,
    void *buf, UInt16 len, UInt8 direction);
```

### 3. Remove `usb_control.rs`

The current `usb_control.rs` and its global-state C functions are replaced entirely:

- `LeptonController` takes an `Arc<UsbStream>` instead of `Arc<UsbControl>`
- Control transfers go through `UsbStream::control_transfer()`
- `wIndex` for UVC extension unit requests: `(unit_id << 8) | interface_number` (low byte = VideoControl interface number, typically 0)

### 4. Modified `camera/acquisition.rs`

Replace `AvCamera` with `UsbStream`:

```rust
pub struct CameraAcquisition {
    stream: Arc<UsbStream>,
    palette: Arc<Mutex<Palette>>,
}
```

- `start()`: calls `stream.start_stream()` with a callback that runs `auto_gain → colorize → emit`
- `stop()`: calls `stream.stop_stream()`
- Remove `CapturedFormat` enum — always Y16
- Remove BGRA path entirely

### 5. Modified `commands/stream.rs`

- `connect_camera`: creates `UsbStream::open()`, stores it in AppState, creates both `CameraAcquisition` and `LeptonController` from the same `Arc<UsbStream>`
- `start_stream`: starts IOKit streaming
- `stop_stream`: stops IOKit streaming
- `set_palette`: unchanged

### 6. Updated `camera/lepton.rs`

- Change constructor to accept `Arc<UsbStream>` instead of `Arc<UsbControl>`
- Internal `get_attribute` / `set_attribute` call `stream.control_transfer()` instead of `usb_control.get_ctrl()` / `usb_control.set_ctrl()`
- Protocol logic (module IDs, control ID calculation, command encoding) unchanged

### 7. Unchanged Components

- `processing/autogain.rs` — receives `&[u8]` (Y16 LE bytes), no changes
- `processing/colorize.rs` — receives grayscale, no changes
- `processing/palettes.rs` — LUT data unchanged
- `commands/controls.rs` — Lepton control commands unchanged
- **All React frontend code** — no changes needed

### 8. Files to Delete

- `src-tauri/src/avfoundation.rs`
- Old global-state functions from `usb_helper.c` (replaced by explicit-handle API)

### 9. Build Changes

- `build.rs`: Remove AVFoundation framework linking; keep IOKit + CoreFoundation
- `Cargo.toml`: Remove dependencies: `objc2-av-foundation`, `objc2-core-media`, `objc2-core-video`, `dispatch2`, `block2`, and any `objc2` crates only used by AVFoundation
- `Cargo.toml`: May need `core-foundation` crate for `CFRunLoop` types in Rust

## UVC Protocol Details

### Y16 Format GUID
```
MEDIASUBTYPE_Y16: {59553136-0000-1000-8000-00aa00389b71}
As bytes (mixed-endian per UUID spec):
  First 4 bytes LE: 36 31 36 59
  Next 2 bytes LE:  00 00
  Next 2 bytes LE:  00 10
  Last 8 bytes BE:  80 00 00 aa 00 38 9b 71
```

### Y16 Pixel Data Endianness
Y16 pixel data is **little-endian** on the USB wire. Each pixel is 2 bytes, LSB first. The existing `autogain.rs` already uses `u16::from_le_bytes()` which is correct.

### UVC Payload Header Format
```
Byte 0: Header length (typically 2 or 12)
Byte 1: Bit field
  - Bit 0 (FID):  Frame ID, toggles each new frame
  - Bit 1 (EOF):  End of frame
  - Bit 2 (PTS):  Presentation timestamp present
  - Bit 3 (SCR):  Source clock reference present
  - Bit 5 (STI):  Still image
  - Bit 6 (ERR):  Error in frame → discard current frame buffer
  - Bit 7 (EOH):  End of header
Bytes 2+: Optional PTS (4 bytes) and SCR (6 bytes)
```

### UVC Video Probe Control (26 bytes minimum)
```
Offset  Size  Field
0       2     bmHint
2       1     bFormatIndex
3       1     bFrameIndex
4       4     dwFrameInterval (100ns units)
8       2     wKeyFrameRate
10      2     wPFrameRate
12      2     wCompQuality
14      2     wCompWindowSize
16      2     wDelay
18      4     dwMaxVideoFrameSize
22      4     dwMaxPayloadTransferSize
```

## Error Handling

Uses existing `CameraError` variants where possible, adds new ones as needed:

| Scenario | Error Variant | Response |
|----------|---------------|----------|
| Device not found | `DeviceNotFound` (existing) | Return error |
| Y16 format not in descriptors | `UvcError("Y16 format not found in descriptors")` | Return error |
| `USBDeviceOpen` fails | `OpenFailed(reason)` (existing) | Return error |
| `USBInterfaceOpen` fails | `OpenFailed(reason)` (existing) | Return error |
| Probe/commit rejected | `UvcError("Probe/commit negotiation failed")` | Return error |
| Isochronous read timeout (>2s) | Internal | Attempt stream restart (re-probe/commit, re-set alt interface) |
| Frame size mismatch (≠ 38400) | Internal | Silent drop, log warning |
| UVC ERR bit set in payload | Internal | Discard current frame buffer |
| Device disconnected during stream | `StreamFailed` (existing) | Emit disconnect event to frontend |

## Testing Strategy

- **Unit tests**: UVC descriptor parsing with known descriptor dumps from PureThermal (capture via `system_profiler SPUSBDataType` or USB packet capture and embed as test fixture)
- **Unit tests**: UVC payload header parsing and frame reassembly logic
- **Unit tests**: Probe/commit structure encoding
- **Integration test**: Connect to real PureThermal, verify Y16 frame dimensions and value ranges
- **Manual test**: Verify palettes, AGC, polarity all work with Y16 data
- **Manual test**: Verify spotmeter temperature readings match known references
