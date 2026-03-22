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
- **Cleanup** — Remove all AVFoundation code

## Architecture

```
PureThermal USB Device (Lepton 3.x sensor)
    │
    ├─ [Video Endpoint - Isochronous]
    │       ↓
    │   IOKit: USBDeviceOpen (exclusive access)
    │       ↓
    │   UVC descriptor parsing → find Y16 format + 160×120 frame
    │       ↓
    │   SetAlternateInterface (activate isoch bandwidth)
    │       ↓
    │   UVC Probe/Commit (negotiate format, frame, interval)
    │       ↓
    │   ReadIsochPipeAsync (continuous isochronous reads)
    │       ↓
    │   UVC payload parsing → frame reassembly (FID bit toggle)
    │       ↓
    │   Complete Y16 frame (160×120×2 = 38,400 bytes)
    │       ↓
    │   auto_gain() → colorize() → RGBA
    │       ↓
    │   Tauri emit("thermal-frame") → React canvas
    │
    └─ [Control Endpoint - Extension Units]
            ↓
        Same device handle (no separate non-exclusive path)
            ↓
        LeptonController: AGC, polarity, FFC, spotmeter, radiometry
```

## Components

### 1. `usb_stream.rs` — New IOKit USB Streaming Module

Replaces `avfoundation.rs`. Written in Rust with FFI calls to an extended `usb_helper.c`.

**Public API:**

```rust
pub struct UsbStream {
    // Internal state: device handle, interface, streaming thread
}

impl UsbStream {
    /// Discover and open the PureThermal device (VID 0x1e4e, PID 0x0100)
    pub fn open() -> Result<Self, CameraError>;

    /// Start Y16 streaming with a frame callback
    pub fn start_stream<F>(&self, callback: F) -> Result<(), CameraError>
    where F: Fn(Y16Frame) + Send + 'static;

    /// Stop streaming, release alt-setting
    pub fn stop_stream(&self) -> Result<(), CameraError>;

    /// Send control transfer (for Lepton commands via UVC extension units)
    pub fn control_transfer(&self, request: ControlRequest) -> Result<Vec<u8>, CameraError>;

    /// Close device, release all resources
    pub fn close(self);
}

pub struct Y16Frame {
    pub data: Vec<u16>,      // 160×120 = 19,200 pixels
    pub width: u32,           // 160
    pub height: u32,          // 120
    pub timestamp: u64,       // USB frame timestamp
}
```

**Internal responsibilities:**

#### 1a. Device Discovery & Open
- Iterate IOKit USB devices matching VID/PID `0x1e4e:0x0100`
- Call `USBDeviceOpen` for exclusive access
- Store device handle for both streaming and control transfers

#### 1b. UVC Descriptor Parsing
- Read configuration descriptor via `GetConfigurationDescriptorPtr`
- Parse UVC VideoStreaming interface descriptors:
  - `VS_FORMAT_UNCOMPRESSED` — find format with Y16 GUID (`59553136-0000-1000-8000-00aa00389b71`)
  - `VS_FRAME_UNCOMPRESSED` — find 160×120 frame descriptor, extract min frame interval
  - Note the format index and frame index for probe/commit
- Find the alt-setting with isochronous endpoint and sufficient `wMaxPacketSize`

#### 1c. UVC Probe/Commit Negotiation
- Send `VS_PROBE_CONTROL SET_CUR` with desired format index, frame index, frame interval
- Read back `VS_PROBE_CONTROL GET_CUR` to confirm negotiated parameters
- Send `VS_COMMIT_CONTROL SET_CUR` to lock parameters
- Structure: UVC Video Probe Control (26 or 34 bytes depending on UVC version)

#### 1d. Stream Activation
- `SetAlternateInterface` to the alt-setting with isochronous bandwidth
- This activates the isochronous endpoint for data transfer

#### 1e. Isochronous Transfer Loop
- Dedicated thread for continuous reads
- Use `ReadIsochPipeAsync` with double-buffered transfer requests
- Each USB microframe delivers a chunk of the current video frame
- Parse UVC payload headers (2+ bytes):
  - Bit 0 (FID): Frame ID — toggles on each new frame boundary
  - Bit 1 (EOF): End of frame marker
  - Bit 5 (STI): Still image flag (ignore)
  - Bytes after header: raw Y16 pixel data
- Accumulate payload data into frame buffer
- On FID toggle or EOF: deliver completed frame, start new buffer
- Drop frames with incorrect size (≠ 38,400 bytes)

#### 1f. Cleanup
- Stop isochronous transfers
- `SetAlternateInterface(0)` to release bandwidth
- `USBDeviceClose`

### 2. Extended `usb_helper.c` — IOKit C Functions

New C functions exposed via FFI (extending existing helper):

```c
// Device lifecycle
IOReturn thermal_usb_open_exclusive(io_service_t device, IOUSBDeviceInterface ***dev);
IOReturn thermal_usb_close_exclusive(IOUSBDeviceInterface **dev);

// Interface management
IOReturn thermal_usb_find_streaming_interface(IOUSBDeviceInterface **dev,
    UInt8 *interface_num, UInt8 *alt_setting, UInt8 *endpoint_addr);
IOReturn thermal_usb_set_alt_interface(IOUSBInterfaceInterface **intf,
    UInt8 alt_setting);

// Descriptor access
IOReturn thermal_usb_get_config_descriptor(IOUSBDeviceInterface **dev,
    UInt8 *buf, UInt16 *len);

// UVC probe/commit
IOReturn thermal_usb_probe_commit(IOUSBInterfaceInterface **intf,
    UvcProbeControl *probe);

// Isochronous streaming
IOReturn thermal_usb_start_isoch(IOUSBInterfaceInterface **intf,
    UInt8 endpoint, IsochCallback callback, void *context);
IOReturn thermal_usb_stop_isoch(IOUSBInterfaceInterface **intf);

// Control transfers (replaces existing non-exclusive version)
IOReturn thermal_usb_ctrl_transfer(IOUSBDeviceInterface **dev,
    UInt8 request, UInt16 value, UInt16 index,
    void *buf, UInt16 len, UInt8 direction);
```

### 3. Simplified `usb_control.rs`

The current `usb_control.rs` wraps C FFI for non-exclusive control transfers. It gets simplified:

- Remove the non-exclusive `DeviceRequest` path
- Use the device handle from `UsbStream` for control transfers
- `UsbControl` becomes a thin wrapper around `UsbStream::control_transfer()`

### 4. Modified `camera/acquisition.rs`

Replace `AvCamera` with `UsbStream`:

```rust
pub struct CameraAcquisition {
    stream: UsbStream,
    palette: Arc<Mutex<Palette>>,
}
```

- `start()`: calls `stream.start_stream()` with a callback that runs `auto_gain → colorize → emit`
- `stop()`: calls `stream.stop_stream()`
- Remove `CapturedFormat` enum — always Y16
- Remove BGRA path entirely

### 5. Modified `commands/stream.rs`

- `connect_camera`: uses `UsbStream::open()` instead of AVFoundation discovery
- `start_stream`: starts IOKit streaming
- `stop_stream`: stops IOKit streaming
- `set_palette`: unchanged

### 6. Unchanged Components

- `camera/lepton.rs` — SDK protocol logic stays the same, transport changes underneath
- `processing/autogain.rs` — receives Y16, no changes
- `processing/colorize.rs` — receives grayscale, no changes
- `processing/palettes.rs` — LUT data unchanged
- `commands/controls.rs` — Lepton control commands unchanged
- **All React frontend code** — no changes needed

### 7. Files to Delete

- `src-tauri/src/avfoundation.rs`

## UVC Protocol Details

### Y16 Format GUID
```
MEDIASUBTYPE_Y16: {59553136-0000-1000-8000-00aa00389b71}
As bytes: 36 31 36 59 00 00 00 10 80 00 00 aa 00 38 9b 71
```

### UVC Payload Header Format
```
Byte 0: Header length (typically 2 or 12)
Byte 1: Bit field
  - Bit 0 (FID):  Frame ID, toggles each new frame
  - Bit 1 (EOF):  End of frame
  - Bit 2 (PTS):  Presentation timestamp present
  - Bit 3 (SCR):  Source clock reference present
  - Bit 5 (STI):  Still image
  - Bit 6 (ERR):  Error in frame
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

| Scenario | Response |
|----------|----------|
| Device not found | Return `CameraError::NotFound` |
| Y16 format not in descriptors | Return `CameraError::UnsupportedFormat` |
| `USBDeviceOpen` fails | Return `CameraError::AccessDenied` |
| Isochronous read timeout (>2s) | Attempt stream restart (re-probe/commit, re-set alt interface) |
| Frame size mismatch | Silent drop, log warning |
| Device disconnected during stream | IOKit notification → emit disconnect event to frontend |
| Probe/commit rejected | Return `CameraError::NegotiationFailed` |

## Testing Strategy

- **Unit tests**: UVC descriptor parsing with known descriptor dumps from PureThermal
- **Unit tests**: UVC payload header parsing and frame reassembly logic
- **Integration test**: Connect to real PureThermal, verify Y16 frame dimensions and value ranges
- **Manual test**: Verify palettes, AGC, polarity all work with Y16 data
- **Manual test**: Verify spotmeter temperature readings match known references
