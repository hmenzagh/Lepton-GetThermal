# Y16 IOKit Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace AVFoundation video capture with direct IOKit isochronous USB transfers to get raw Y16 thermal data from PureThermal.

**Architecture:** A new `usb_stream.rs` module replaces `avfoundation.rs`, using IOKit C FFI (extended `usb_helper.c`) for exclusive USB device access, UVC descriptor negotiation, and isochronous frame transfer. `LeptonController` and `CameraAcquisition` both use the shared `UsbStream` handle. Two pure-Rust parsing modules (`uvc_descriptors.rs`, `uvc_payload.rs`) handle UVC protocol logic with full test coverage.

**Tech Stack:** Rust, C (IOKit FFI), Tauri v2, React/TypeScript (frontend unchanged)

**Spec:** `docs/superpowers/specs/2026-03-22-y16-iokit-streaming-design.md`

**Important implementation notes:**
- The Y16 GUID used in descriptor parsing MUST be verified against the real PureThermal device descriptor before relying on it. Run `system_profiler SPUSBDataType -xml` or capture a USB trace to get the actual GUID. The plan uses the standard MEDIASUBTYPE_Y16 GUID `{20363159-0000-0010-8000-00AA00389B71}` (FOURCC "Y16 ") as default.
- The IOKit C helper must query `kIOUSBInterfaceInterfaceID190` (not the base `kIOUSBInterfaceInterfaceID`) to get an interface that supports `ReadIsochPipeAsync`.
- `wMaxPacketSize` in USB 2.0 HS isochronous endpoints uses bits 12:11 for additional transactions per microframe. The actual payload per microframe = `(wMaxPacketSize & 0x7FF) * ((wMaxPacketSize >> 11 & 3) + 1)`.
- Probe/Commit control transfers go through `DeviceRequest` on the device handle (macOS routes class/interface requests to the correct interface based on `wIndex`). This is simpler than opening the VS interface for control transfers separately.

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `src-tauri/src/uvc_descriptors.rs` | Parse UVC configuration descriptors to find Y16 format, frame, and alt-setting |
| `src-tauri/src/uvc_payload.rs` | Parse UVC payload headers, reassemble isochronous packets into complete Y16 frames |
| `src-tauri/src/usb_stream.rs` | IOKit USB streaming: device open, interface lifecycle, isoch transfers, control transfers |

### Modified files
| File | Changes |
|------|---------|
| `src-tauri/src/usb_helper.c` | Replace global-state API with explicit-handle functions: exclusive open, interface lifecycle, isoch transfers, probe/commit |
| `src-tauri/src/camera/lepton.rs` | Change `Arc<UsbControl>` → `Arc<UsbStream>`, use `stream.control_transfer()` |
| `src-tauri/src/camera/acquisition.rs` | Replace `AvCamera` with `Arc<UsbStream>`, remove BGRA path |
| `src-tauri/src/commands/stream.rs` | Create `UsbStream` in `connect_camera`, share with both `CameraAcquisition` and `LeptonController` |
| `src-tauri/src/lib.rs` | Remove `mod avfoundation`, `mod usb_control`; add `mod usb_stream`, `mod uvc_descriptors`, `mod uvc_payload` |
| `src-tauri/src/camera/types.rs` | No changes (existing error variants suffice) |
| `src-tauri/build.rs` | Remove AVFoundation framework linking |
| `src-tauri/Cargo.toml` | Remove objc2/AVFoundation/block2/dispatch2 dependencies |

### Deleted files
| File | Reason |
|------|--------|
| `src-tauri/src/avfoundation.rs` | Replaced by `usb_stream.rs` |
| `src-tauri/src/usb_control.rs` | Replaced by `UsbStream::control_transfer()` |

---

## Task 1: UVC Descriptor Parsing Module

Pure Rust module that parses USB configuration descriptors to find the Y16 video format, frame descriptor, and appropriate alt-setting. Fully testable without hardware.

**Files:**
- Create: `src-tauri/src/uvc_descriptors.rs`

- [ ] **Step 1: Write failing tests for descriptor parsing**

Create `src-tauri/src/uvc_descriptors.rs` with types and tests only:

```rust
//! UVC configuration descriptor parsing.
//! Extracts Y16 format index, frame index, frame interval,
//! and isochronous alt-setting from USB configuration descriptors.

/// Y16 format GUID (mixed-endian per USB/UVC spec)
/// MEDIASUBTYPE_Y16: {20363159-0000-0010-8000-00AA00389B71}
/// FOURCC "Y16 " (0x59, 0x31, 0x36, 0x20)
/// NOTE: Verify against real PureThermal descriptor dump! The firmware
/// may use a variant GUID (e.g., null-terminated instead of space-padded).
const Y16_GUID: [u8; 16] = [
    0x59, 0x31, 0x36, 0x20, // FOURCC "Y16 " stored as raw bytes
    0x00, 0x00,             // next 2 bytes LE
    0x10, 0x00,             // next 2 bytes LE
    0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71, // last 8 bytes BE
];

/// UVC descriptor subtypes (VideoStreaming)
const VS_FORMAT_UNCOMPRESSED: u8 = 0x04;
const VS_FRAME_UNCOMPRESSED: u8 = 0x05;

/// Result of parsing UVC descriptors for Y16 streaming.
#[derive(Debug, Clone, PartialEq)]
pub struct UvcStreamConfig {
    /// VideoStreaming interface number
    pub vs_interface_num: u8,
    /// Format index for Y16 in VS_FORMAT_UNCOMPRESSED
    pub format_index: u8,
    /// Frame index for desired resolution in VS_FRAME_UNCOMPRESSED
    pub frame_index: u8,
    /// Frame width
    pub width: u16,
    /// Frame height
    pub height: u16,
    /// Default frame interval in 100ns units
    pub frame_interval: u32,
    /// Alt-setting number with sufficient isochronous bandwidth
    pub alt_setting: u8,
    /// Isochronous endpoint address
    pub endpoint_addr: u8,
    /// Raw wMaxPacketSize for the chosen alt-setting (includes HS mult bits 12:11)
    pub max_packet_size: u16,
    /// Bits per pixel (16 for Y16)
    pub bits_per_pixel: u8,
}

impl UvcStreamConfig {
    /// Effective bytes per microframe, accounting for USB 2.0 HS multiplier.
    /// wMaxPacketSize bits 10:0 = packet size, bits 12:11 = additional transactions.
    pub fn effective_max_packet(&self) -> usize {
        let pkt_size = (self.max_packet_size & 0x7FF) as usize;
        let mult = ((self.max_packet_size >> 11) & 0x3) as usize + 1;
        pkt_size * mult
    }
}

/// Parse a USB configuration descriptor to find Y16 streaming config.
pub fn parse_uvc_config(_descriptor: &[u8]) -> Result<UvcStreamConfig, String> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal USB config descriptor with:
    /// - Interface 0 (VideoControl, placeholder)
    /// - Interface 1 (VideoStreaming, alt 0)
    /// - VS_FORMAT_UNCOMPRESSED with Y16 GUID, format index 2
    /// - VS_FRAME_UNCOMPRESSED 160x120, frame interval 370370 (27fps)
    /// - Interface 1 alt 1 with isoch endpoint 0x81, max packet 956
    /// NOTE: Uses the standard Y16 GUID {20363159-0000-0010-8000-00AA00389B71}.
    /// If the real PureThermal uses a different GUID, update both here and Y16_GUID const.
    fn make_test_descriptor() -> Vec<u8> {
        let mut desc = Vec::new();

        // === Configuration descriptor header (9 bytes) ===
        desc.extend_from_slice(&[
            0x09, // bLength
            0x02, // bDescriptorType = CONFIGURATION
            0x00, 0x00, // wTotalLength (patched later)
            0x02, // bNumInterfaces
            0x01, // bConfigurationValue
            0x00, // iConfiguration
            0x80, // bmAttributes
            0xFA, // bMaxPower
        ]);

        // === Interface 0 (VideoControl) — skip, we don't parse it ===
        desc.extend_from_slice(&[
            0x09, 0x04, // bLength, bDescriptorType = INTERFACE
            0x00, // bInterfaceNumber = 0
            0x00, // bAlternateSetting = 0
            0x01, // bNumEndpoints
            0x0E, // bInterfaceClass = Video
            0x01, // bInterfaceSubClass = VideoControl
            0x00, // bInterfaceProtocol
            0x00, // iInterface
        ]);

        // === Interface 1 (VideoStreaming, alt 0 = zero bandwidth) ===
        desc.extend_from_slice(&[
            0x09, 0x04,
            0x01, // bInterfaceNumber = 1
            0x00, // bAlternateSetting = 0
            0x00, // bNumEndpoints = 0 (zero bandwidth)
            0x0E, // bInterfaceClass = Video
            0x02, // bInterfaceSubClass = VideoStreaming
            0x00, 0x00,
        ]);

        // === VS_FORMAT_UNCOMPRESSED (27 bytes) ===
        desc.push(0x1B); // bLength = 27
        desc.push(0x24); // bDescriptorType = CS_INTERFACE
        desc.push(VS_FORMAT_UNCOMPRESSED); // bDescriptorSubtype
        desc.push(0x02); // bFormatIndex = 2
        desc.push(0x01); // bNumFrameDescriptors
        desc.extend_from_slice(&Y16_GUID); // guidFormat
        desc.push(16);   // bBitsPerPixel = 16
        desc.push(0x01); // bDefaultFrameIndex
        desc.push(0x00); // bAspectRatioX
        desc.push(0x00); // bAspectRatioY
        desc.push(0x00); // bmInterlaceFlags
        desc.push(0x00); // bCopyProtect

        // === VS_FRAME_UNCOMPRESSED (30 bytes with 1 interval) ===
        desc.push(30);   // bLength
        desc.push(0x24); // bDescriptorType = CS_INTERFACE
        desc.push(VS_FRAME_UNCOMPRESSED); // bDescriptorSubtype
        desc.push(0x01); // bFrameIndex = 1
        desc.push(0x00); // bmCapabilities
        desc.extend_from_slice(&160u16.to_le_bytes()); // wWidth
        desc.extend_from_slice(&120u16.to_le_bytes()); // wHeight
        desc.extend_from_slice(&3_072_000u32.to_le_bytes()); // dwMinBitRate
        desc.extend_from_slice(&3_072_000u32.to_le_bytes()); // dwMaxBitRate
        desc.extend_from_slice(&38_400u32.to_le_bytes());    // dwMaxVideoFrameBufferSize
        desc.extend_from_slice(&370_370u32.to_le_bytes());   // dwDefaultFrameInterval
        desc.push(0x01); // bFrameIntervalType = 1 discrete
        desc.extend_from_slice(&370_370u32.to_le_bytes());   // dwFrameInterval[0]

        // === Interface 1 alt 1 (with isoch endpoint) ===
        desc.extend_from_slice(&[
            0x09, 0x04,
            0x01, // bInterfaceNumber = 1
            0x01, // bAlternateSetting = 1
            0x01, // bNumEndpoints = 1
            0x0E, 0x02, 0x00, 0x00,
        ]);

        // === Isochronous endpoint descriptor (7 bytes) ===
        desc.extend_from_slice(&[
            0x07, // bLength
            0x05, // bDescriptorType = ENDPOINT
            0x81, // bEndpointAddress = 1 IN
            0x05, // bmAttributes = isochronous, async
        ]);
        desc.extend_from_slice(&956u16.to_le_bytes()); // wMaxPacketSize
        desc.push(0x01); // bInterval

        // Patch wTotalLength
        let total = desc.len() as u16;
        desc[2] = (total & 0xFF) as u8;
        desc[3] = (total >> 8) as u8;

        desc
    }

    #[test]
    fn parse_finds_y16_format() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).unwrap();
        assert_eq!(config.format_index, 2);
        assert_eq!(config.bits_per_pixel, 16);
    }

    #[test]
    fn parse_finds_frame_dimensions() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).unwrap();
        assert_eq!(config.width, 160);
        assert_eq!(config.height, 120);
        assert_eq!(config.frame_index, 1);
    }

    #[test]
    fn parse_finds_frame_interval() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).unwrap();
        assert_eq!(config.frame_interval, 370_370);
    }

    #[test]
    fn parse_finds_isoch_endpoint() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).unwrap();
        assert_eq!(config.vs_interface_num, 1);
        assert_eq!(config.alt_setting, 1);
        assert_eq!(config.endpoint_addr, 0x81);
        assert_eq!(config.max_packet_size, 956);
    }

    #[test]
    fn parse_rejects_missing_y16() {
        // Descriptor with no Y16 format → should fail
        let mut desc = make_test_descriptor();
        // Corrupt the GUID (offset depends on structure, find the GUID bytes)
        // The GUID starts after the VS_FORMAT header (3 bytes: len, type, subtype, formatIndex, numFrameDesc)
        // Find it by searching for Y16_GUID in the descriptor
        if let Some(pos) = desc.windows(16).position(|w| w == Y16_GUID) {
            desc[pos] = 0xFF; // corrupt first byte
        }
        assert!(parse_uvc_config(&desc).is_err());
    }
}
```

- [ ] **Step 2: Register module in lib.rs and run tests to verify they fail**

Add `mod uvc_descriptors;` temporarily to `src-tauri/src/lib.rs` (line 4, after `mod processing;`):

```rust
mod uvc_descriptors;
```

Run:
```bash
cd src-tauri && cargo test uvc_descriptors -- --nocapture
```

Expected: FAIL — `todo!()` panics.

- [ ] **Step 3: Implement `parse_uvc_config`**

Replace the `todo!()` in `parse_uvc_config` with the actual implementation:

```rust
/// Parse a USB configuration descriptor to find Y16 streaming config.
pub fn parse_uvc_config(descriptor: &[u8]) -> Result<UvcStreamConfig, String> {
    // Skip the 9-byte configuration descriptor header
    if descriptor.len() < 9 {
        return Err("Descriptor too short".into());
    }

    let total_len = u16::from_le_bytes([descriptor[2], descriptor[3]]) as usize;
    if descriptor.len() < total_len {
        return Err("Descriptor truncated".into());
    }

    let mut y16_format_index: Option<u8> = None;
    let mut bits_per_pixel: u8 = 0;
    let mut frame_index: Option<u8> = None;
    let mut width: u16 = 0;
    let mut height: u16 = 0;
    let mut frame_interval: u32 = 0;
    let mut vs_interface_num: Option<u8> = None;
    let mut best_alt_setting: Option<u8> = None;
    let mut best_endpoint: u8 = 0;
    let mut best_max_packet: u16 = 0;

    // Track current interface context while parsing
    let mut current_interface_num: u8 = 0;
    let mut current_alt_setting: u8 = 0;
    let mut in_vs_interface = false;

    let mut pos = 0;
    while pos + 1 < total_len {
        let b_length = descriptor[pos] as usize;
        if b_length < 2 || pos + b_length > total_len {
            break;
        }
        let b_type = descriptor[pos + 1];

        match b_type {
            // Standard interface descriptor
            0x04 if b_length >= 9 => {
                current_interface_num = descriptor[pos + 2];
                current_alt_setting = descriptor[pos + 3];
                let iface_class = descriptor[pos + 5];
                let iface_subclass = descriptor[pos + 6];
                in_vs_interface = iface_class == 0x0E && iface_subclass == 0x02;
                if in_vs_interface && vs_interface_num.is_none() {
                    vs_interface_num = Some(current_interface_num);
                }
            }
            // CS_INTERFACE (class-specific)
            0x24 if b_length >= 3 && in_vs_interface => {
                let subtype = descriptor[pos + 2];
                match subtype {
                    // VS_FORMAT_UNCOMPRESSED
                    s if s == VS_FORMAT_UNCOMPRESSED && b_length >= 27 => {
                        let fmt_index = descriptor[pos + 3];
                        let guid = &descriptor[pos + 5..pos + 21];
                        if guid == Y16_GUID {
                            y16_format_index = Some(fmt_index);
                            bits_per_pixel = descriptor[pos + 21];
                        }
                    }
                    // VS_FRAME_UNCOMPRESSED
                    s if s == VS_FRAME_UNCOMPRESSED && b_length >= 26 => {
                        if y16_format_index.is_some() && frame_index.is_none() {
                            frame_index = Some(descriptor[pos + 3]);
                            width = u16::from_le_bytes([descriptor[pos + 5], descriptor[pos + 6]]);
                            height = u16::from_le_bytes([descriptor[pos + 7], descriptor[pos + 8]]);
                            frame_interval = u32::from_le_bytes([
                                descriptor[pos + 21], descriptor[pos + 22],
                                descriptor[pos + 23], descriptor[pos + 24],
                            ]);
                        }
                    }
                    _ => {}
                }
            }
            // Endpoint descriptor
            0x05 if b_length >= 7 && in_vs_interface && current_alt_setting > 0 => {
                let addr = descriptor[pos + 2];
                let attrs = descriptor[pos + 3];
                let max_pkt = u16::from_le_bytes([descriptor[pos + 4], descriptor[pos + 5]]);
                // Check: IN endpoint (bit 7) and isochronous (bits 0-1 = 01)
                if (addr & 0x80) != 0 && (attrs & 0x03) == 0x01 {
                    // Pick the alt-setting with largest packet size
                    if max_pkt > best_max_packet {
                        best_alt_setting = Some(current_alt_setting);
                        best_endpoint = addr;
                        best_max_packet = max_pkt;
                    }
                }
            }
            _ => {}
        }

        pos += b_length;
    }

    let format_index = y16_format_index.ok_or("Y16 format not found in descriptors")?;
    let frame_idx = frame_index.ok_or("No frame descriptor found for Y16 format")?;
    let vs_iface = vs_interface_num.ok_or("No VideoStreaming interface found")?;
    let alt = best_alt_setting.ok_or("No isochronous alt-setting found")?;

    Ok(UvcStreamConfig {
        vs_interface_num: vs_iface,
        format_index,
        frame_index: frame_idx,
        width,
        height,
        frame_interval,
        alt_setting: alt,
        endpoint_addr: best_endpoint,
        max_packet_size: best_max_packet,
        bits_per_pixel,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd src-tauri && cargo test uvc_descriptors -- --nocapture
```

Expected: All 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/uvc_descriptors.rs src-tauri/src/lib.rs
git commit -m "feat: add UVC descriptor parsing module with Y16 format detection"
```

---

## Task 2: UVC Payload Parsing Module

Pure Rust module for parsing UVC isochronous payload headers and reassembling packets into complete Y16 frames. Fully testable without hardware.

**Files:**
- Create: `src-tauri/src/uvc_payload.rs`

- [ ] **Step 1: Write failing tests for payload parsing**

Create `src-tauri/src/uvc_payload.rs`:

```rust
//! UVC isochronous payload parsing and frame reassembly.
//! Parses UVC payload headers from isochronous transfer data and
//! assembles complete Y16 video frames.

/// UVC payload header bit flags (byte 1)
const BFH_FID: u8 = 0x01;  // Frame ID — toggles on new frame
const BFH_EOF: u8 = 0x02;  // End of frame
const BFH_PTS: u8 = 0x04;  // PTS present
const BFH_SCR: u8 = 0x08;  // SCR present
const BFH_STI: u8 = 0x20;  // Still image
const BFH_ERR: u8 = 0x40;  // Error in frame
const BFH_EOH: u8 = 0x80;  // End of header

/// Parsed UVC payload header.
#[derive(Debug, Clone, PartialEq)]
pub struct UvcPayloadHeader {
    pub header_len: u8,
    pub fid: bool,
    pub eof: bool,
    pub err: bool,
    pub pts: Option<u32>,
    pub scr: Option<[u8; 6]>,
}

/// Parse a UVC payload header from raw bytes.
/// Returns the header and the offset where payload data begins.
pub fn parse_payload_header(_data: &[u8]) -> Result<(UvcPayloadHeader, usize), String> {
    todo!()
}

/// Assembles complete Y16 frames from a stream of UVC payload packets.
pub struct FrameAssembler {
    buffer: Vec<u8>,
    expected_size: usize,
    last_fid: Option<bool>,
}

impl FrameAssembler {
    /// Create a new assembler for frames of `width * height * 2` bytes.
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            buffer: Vec::with_capacity((width as usize) * (height as usize) * 2),
            expected_size: (width as usize) * (height as usize) * 2,
            last_fid: None,
        }
    }

    /// Feed a raw isochronous packet. Returns a complete frame if one is ready.
    pub fn feed(&mut self, _packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
        todo!()
    }

    /// Reset the assembler state (e.g., after error).
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.last_fid = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_header(fid: bool, eof: bool, err: bool) -> Vec<u8> {
        let mut flags: u8 = BFH_EOH; // always set EOH
        if fid { flags |= BFH_FID; }
        if eof { flags |= BFH_EOF; }
        if err { flags |= BFH_ERR; }
        vec![2, flags] // 2-byte header (minimal)
    }

    fn make_packet(fid: bool, eof: bool, payload: &[u8]) -> Vec<u8> {
        let mut pkt = make_header(fid, eof, false);
        pkt.extend_from_slice(payload);
        pkt
    }

    #[test]
    fn parse_minimal_header() {
        let data = make_header(true, false, false);
        let (hdr, offset) = parse_payload_header(&data).unwrap();
        assert!(hdr.fid);
        assert!(!hdr.eof);
        assert!(!hdr.err);
        assert_eq!(hdr.header_len, 2);
        assert_eq!(offset, 2);
        assert!(hdr.pts.is_none());
    }

    #[test]
    fn parse_header_with_pts() {
        let mut data = vec![12, BFH_PTS | BFH_SCR | BFH_EOH];
        // PTS: 4 bytes
        data.extend_from_slice(&42u32.to_le_bytes());
        // SCR: 6 bytes
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let (hdr, offset) = parse_payload_header(&data).unwrap();
        assert_eq!(hdr.pts, Some(42));
        assert!(hdr.scr.is_some());
        assert_eq!(offset, 12);
    }

    #[test]
    fn parse_header_error_flag() {
        let data = make_header(false, false, true);
        let (hdr, _) = parse_payload_header(&data).unwrap();
        assert!(hdr.err);
    }

    #[test]
    fn parse_empty_data_fails() {
        assert!(parse_payload_header(&[]).is_err());
        assert!(parse_payload_header(&[1]).is_err());
    }

    #[test]
    fn assemble_single_packet_frame() {
        // 2x2 Y16 frame = 8 bytes
        let mut asm = FrameAssembler::new(2, 2);
        let payload = vec![0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00];
        let pkt = make_packet(true, true, &payload);
        let frame = asm.feed(&pkt).unwrap();
        assert!(frame.is_some());
        assert_eq!(frame.unwrap(), payload);
    }

    #[test]
    fn assemble_multi_packet_frame() {
        // 2x2 frame split across 2 packets
        let mut asm = FrameAssembler::new(2, 2);

        let pkt1 = make_packet(true, false, &[0x01, 0x00, 0x02, 0x00]);
        assert!(asm.feed(&pkt1).unwrap().is_none()); // not complete yet

        let pkt2 = make_packet(true, true, &[0x03, 0x00, 0x04, 0x00]);
        let frame = asm.feed(&pkt2).unwrap();
        assert!(frame.is_some());
        assert_eq!(frame.unwrap().len(), 8);
    }

    #[test]
    fn assemble_discards_on_error() {
        let mut asm = FrameAssembler::new(2, 2);

        let pkt1 = make_packet(true, false, &[0x01, 0x00, 0x02, 0x00]);
        asm.feed(&pkt1).unwrap();

        // Error packet → discard buffer
        let mut err_pkt = make_header(true, false, true);
        err_pkt.extend_from_slice(&[0xFF, 0xFF]);
        let result = asm.feed(&err_pkt).unwrap();
        assert!(result.is_none());

        // Next frame starts fresh
        let pkt_new = make_packet(false, true, &[0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00]);
        let frame = asm.feed(&pkt_new).unwrap();
        assert!(frame.is_some());
    }

    #[test]
    fn assemble_fid_toggle_starts_new_frame() {
        let mut asm = FrameAssembler::new(2, 2);

        // Start frame with FID=true, but don't finish (no EOF)
        let pkt1 = make_packet(true, false, &[0x01, 0x00, 0x02, 0x00]);
        asm.feed(&pkt1).unwrap();

        // FID toggles to false → previous frame discarded, new frame starts
        let pkt2 = make_packet(false, true, &[0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00]);
        let frame = asm.feed(&pkt2).unwrap();
        assert!(frame.is_some());
        assert_eq!(frame.unwrap().len(), 8);
    }

    #[test]
    fn assemble_drops_wrong_size_frame() {
        let mut asm = FrameAssembler::new(2, 2); // expects 8 bytes

        // Send only 6 bytes then EOF → frame dropped (wrong size)
        let pkt = make_packet(true, true, &[0x01, 0x00, 0x02, 0x00, 0x03, 0x00]);
        let frame = asm.feed(&pkt).unwrap();
        assert!(frame.is_none()); // dropped because 6 != 8
    }
}
```

- [ ] **Step 2: Register module and run tests to verify they fail**

Add `mod uvc_payload;` to `src-tauri/src/lib.rs` (after `mod uvc_descriptors;`).

```bash
cd src-tauri && cargo test uvc_payload -- --nocapture
```

Expected: FAIL — `todo!()` panics.

- [ ] **Step 3: Implement `parse_payload_header`**

Replace the `todo!()` in `parse_payload_header`:

```rust
pub fn parse_payload_header(data: &[u8]) -> Result<(UvcPayloadHeader, usize), String> {
    if data.len() < 2 {
        return Err("Payload too short for UVC header".into());
    }

    let header_len = data[0];
    if (header_len as usize) > data.len() || header_len < 2 {
        return Err(format!("Invalid header length: {header_len}"));
    }

    let flags = data[1];
    let fid = (flags & BFH_FID) != 0;
    let eof = (flags & BFH_EOF) != 0;
    let err = (flags & BFH_ERR) != 0;
    let has_pts = (flags & BFH_PTS) != 0;
    let has_scr = (flags & BFH_SCR) != 0;

    let mut offset = 2usize;

    let pts = if has_pts && offset + 4 <= header_len as usize {
        let val = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
        offset += 4;
        Some(val)
    } else {
        None
    };

    let scr = if has_scr && offset + 6 <= header_len as usize {
        let mut s = [0u8; 6];
        s.copy_from_slice(&data[offset..offset+6]);
        offset += 6;
        Some(s)
    } else {
        None
    };

    Ok((
        UvcPayloadHeader { header_len, fid, eof, err, pts, scr },
        header_len as usize,
    ))
}
```

- [ ] **Step 4: Implement `FrameAssembler::feed`**

Replace the `todo!()` in `feed`:

```rust
pub fn feed(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let (header, data_offset) = parse_payload_header(packet)?;

    // Error flag → discard current frame
    if header.err {
        self.buffer.clear();
        self.last_fid = Some(header.fid);
        return Ok(None);
    }

    // FID toggle → new frame boundary
    if let Some(last) = self.last_fid {
        if last != header.fid && !self.buffer.is_empty() {
            // Previous frame was incomplete, discard it
            self.buffer.clear();
        }
    }
    self.last_fid = Some(header.fid);

    // Append payload data
    if data_offset < packet.len() {
        self.buffer.extend_from_slice(&packet[data_offset..]);
    }

    // Check for complete frame
    if header.eof {
        if self.buffer.len() == self.expected_size {
            let frame = std::mem::take(&mut self.buffer);
            self.buffer.reserve(self.expected_size);
            return Ok(Some(frame));
        } else {
            // Wrong size → drop
            eprintln!(
                "[uvc] Frame size mismatch: got {} expected {}",
                self.buffer.len(),
                self.expected_size
            );
            self.buffer.clear();
            return Ok(None);
        }
    }

    Ok(None)
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd src-tauri && cargo test uvc_payload -- --nocapture
```

Expected: All 8 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/uvc_payload.rs src-tauri/src/lib.rs
git commit -m "feat: add UVC payload parsing and frame reassembly module"
```

---

## Task 3: Rewrite `usb_helper.c` with Explicit-Handle IOKit API

Replace the global-state C helper with explicit-handle functions for exclusive device access, interface lifecycle, isochronous transfers, and probe/commit.

**Files:**
- Modify: `src-tauri/src/usb_helper.c` (full rewrite)
- Modify: `src-tauri/build.rs`

- [ ] **Step 1: Rewrite `usb_helper.c`**

Replace the entire file with:

```c
/*
 * IOKit USB helper for UVC streaming and control transfers.
 * Uses explicit handles (no global state). Supports:
 * - Exclusive device open/close
 * - Interface discovery, open, alt-setting
 * - Isochronous transfer setup
 * - Control transfers (UVC extension units + class-specific)
 * - Configuration descriptor access
 */

#include <IOKit/IOKitLib.h>
#include <IOKit/usb/IOUSBLib.h>
#include <IOKit/IOCFPlugIn.h>
#include <CoreFoundation/CoreFoundation.h>
#include <mach/mach.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* ---------- Device lifecycle ---------- */

int thermal_usb_find_device(uint16_t vid, uint16_t pid, io_service_t *service_out) {
    CFMutableDictionaryRef matching = IOServiceMatching(kIOUSBDeviceClassName);
    if (!matching) return -1;

    CFNumberRef vidNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &(int32_t){vid});
    CFNumberRef pidNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &(int32_t){pid});
    CFDictionarySetValue(matching, CFSTR(kUSBVendorID), vidNum);
    CFDictionarySetValue(matching, CFSTR(kUSBProductID), pidNum);
    CFRelease(vidNum);
    CFRelease(pidNum);

    io_iterator_t iterator = 0;
    kern_return_t kr = IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator);
    if (kr != KERN_SUCCESS) return -2;

    *service_out = IOIteratorNext(iterator);
    IOObjectRelease(iterator);
    if (!*service_out) return -3;
    return 0;
}

int thermal_usb_open_device(io_service_t service, IOUSBDeviceInterface ***dev_out) {
    IOCFPlugInInterface **plugIn = NULL;
    SInt32 score = 0;
    kern_return_t kr = IOCreatePlugInInterfaceForService(
        service, kIOUSBDeviceUserClientTypeID, kIOCFPlugInInterfaceID, &plugIn, &score);
    IOObjectRelease(service);
    if (kr != KERN_SUCCESS || !plugIn) return -4;

    IOUSBDeviceInterface **dev = NULL;
    HRESULT hr = (*plugIn)->QueryInterface(plugIn,
        CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID), (LPVOID *)&dev);
    (*plugIn)->Release(plugIn);
    if (hr != 0 || !dev) return -5;

    kr = (*dev)->USBDeviceOpen(dev);
    if (kr != KERN_SUCCESS) {
        (*dev)->Release(dev);
        return -6;
    }

    *dev_out = dev;
    return 0;
}

int thermal_usb_set_configuration(IOUSBDeviceInterface **dev, uint8_t config) {
    return (*dev)->SetConfiguration(dev, config);
}

void thermal_usb_close_device(IOUSBDeviceInterface **dev) {
    if (dev) {
        (*dev)->USBDeviceClose(dev);
        (*dev)->Release(dev);
    }
}

/* ---------- Interface lifecycle ---------- */

int thermal_usb_find_interface(IOUSBDeviceInterface **dev,
    uint8_t iface_class, uint8_t iface_subclass,
    IOUSBInterfaceInterface ***intf_out)
{
    IOUSBFindInterfaceRequest req;
    req.bInterfaceClass = iface_class;
    req.bInterfaceSubClass = iface_subclass;
    req.bInterfaceProtocol = kIOUSBFindInterfaceDontCare;
    req.bAlternateSetting = kIOUSBFindInterfaceDontCare;

    io_iterator_t iterator = 0;
    kern_return_t kr = (*dev)->CreateInterfaceIterator(dev, &req, &iterator);
    if (kr != KERN_SUCCESS) return -1;

    io_service_t intf_service = IOIteratorNext(iterator);
    IOObjectRelease(iterator);
    if (!intf_service) return -2;

    IOCFPlugInInterface **plugIn = NULL;
    SInt32 score = 0;
    kr = IOCreatePlugInInterfaceForService(intf_service,
        kIOUSBInterfaceUserClientTypeID, kIOCFPlugInInterfaceID, &plugIn, &score);
    IOObjectRelease(intf_service);
    if (kr != KERN_SUCCESS || !plugIn) return -3;

    /* Must use ID190 or later to get ReadIsochPipeAsync support */
    IOUSBInterfaceInterface190 **intf = NULL;
    HRESULT hr = (*plugIn)->QueryInterface(plugIn,
        CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID190), (LPVOID *)&intf);
    (*plugIn)->Release(plugIn);
    if (hr != 0 || !intf) return -4;

    *intf_out = intf;
    return 0;
}

int thermal_usb_open_interface(IOUSBInterfaceInterface **intf) {
    return (*intf)->USBInterfaceOpen(intf);
}

void thermal_usb_close_interface(IOUSBInterfaceInterface **intf) {
    if (intf) {
        (*intf)->USBInterfaceClose(intf);
        (*intf)->Release(intf);
    }
}

int thermal_usb_set_alt_interface(IOUSBInterfaceInterface **intf, uint8_t alt) {
    return (*intf)->SetAlternateInterface(intf, alt);
}

/* ---------- Configuration descriptor ---------- */

int thermal_usb_get_config_desc(IOUSBDeviceInterface **dev, uint8_t *buf, uint16_t *len) {
    IOUSBConfigurationDescriptorPtr config = NULL;
    kern_return_t kr = (*dev)->GetConfigurationDescriptorPtr(dev, 0, &config);
    if (kr != KERN_SUCCESS) return -1;

    uint16_t total = config->wTotalLength;
    if (total > *len) total = *len;
    memcpy(buf, config, total);
    *len = total;
    return 0;
}

/* ---------- Control transfers ---------- */

int thermal_usb_device_request(IOUSBDeviceInterface **dev,
    uint8_t bmRequestType, uint8_t bRequest,
    uint16_t wValue, uint16_t wIndex,
    void *buf, uint16_t wLength, uint16_t *actual_len)
{
    IOUSBDevRequest req;
    req.bmRequestType = bmRequestType;
    req.bRequest = bRequest;
    req.wValue = wValue;
    req.wIndex = wIndex;
    req.wLength = wLength;
    req.pData = buf;
    req.wLenDone = 0;

    kern_return_t kr = (*dev)->DeviceRequest(dev, &req);
    if (kr != KERN_SUCCESS) return -(int)kr;
    if (actual_len) *actual_len = req.wLenDone;
    return 0;
}

/* UVC extension unit GET_CUR / SET_CUR (convenience wrappers) */

int thermal_usb_get_ctrl(IOUSBDeviceInterface **dev,
    uint8_t unit_id, uint8_t control_id,
    uint8_t *data, uint16_t length)
{
    uint16_t actual = 0;
    int ret = thermal_usb_device_request(dev,
        USBmakebmRequestType(kUSBIn, kUSBClass, kUSBInterface),
        0x81, /* GET_CUR */
        (uint16_t)control_id << 8,
        (uint16_t)unit_id << 8,
        data, length, &actual);
    return (ret == 0) ? (int)actual : ret;
}

int thermal_usb_set_ctrl(IOUSBDeviceInterface **dev,
    uint8_t unit_id, uint8_t control_id,
    const uint8_t *data, uint16_t length)
{
    return thermal_usb_device_request(dev,
        USBmakebmRequestType(kUSBOut, kUSBClass, kUSBInterface),
        0x01, /* SET_CUR */
        (uint16_t)control_id << 8,
        (uint16_t)unit_id << 8,
        (void *)data, length, NULL);
}

/* ---------- UVC Probe/Commit ---------- */

int thermal_usb_vs_probe_set(IOUSBDeviceInterface **dev,
    uint8_t vs_iface_num, void *probe_data, uint16_t len)
{
    return thermal_usb_device_request(dev,
        USBmakebmRequestType(kUSBOut, kUSBClass, kUSBInterface),
        0x01, /* SET_CUR */
        (0x01 << 8), /* VS_PROBE_CONTROL */
        vs_iface_num,
        probe_data, len, NULL);
}

int thermal_usb_vs_probe_get(IOUSBDeviceInterface **dev,
    uint8_t vs_iface_num, void *probe_data, uint16_t len)
{
    return thermal_usb_device_request(dev,
        USBmakebmRequestType(kUSBIn, kUSBClass, kUSBInterface),
        0x81, /* GET_CUR */
        (0x01 << 8), /* VS_PROBE_CONTROL */
        vs_iface_num,
        probe_data, len, NULL);
}

int thermal_usb_vs_commit(IOUSBDeviceInterface **dev,
    uint8_t vs_iface_num, void *probe_data, uint16_t len)
{
    return thermal_usb_device_request(dev,
        USBmakebmRequestType(kUSBOut, kUSBClass, kUSBInterface),
        0x01, /* SET_CUR */
        (0x02 << 8), /* VS_COMMIT_CONTROL */
        vs_iface_num,
        probe_data, len, NULL);
}

/* ---------- Isochronous streaming ---------- */

int thermal_usb_get_pipe_ref(IOUSBInterfaceInterface **intf,
    uint8_t endpoint_addr, uint8_t *pipe_ref_out)
{
    UInt8 num_endpoints = 0;
    kern_return_t kr = (*intf)->GetNumEndpoints(intf, &num_endpoints);
    if (kr != KERN_SUCCESS) return -1;

    for (UInt8 i = 1; i <= num_endpoints; i++) {
        UInt8 direction, number, transferType, interval;
        UInt16 maxPacketSize;
        kr = (*intf)->GetPipeProperties(intf, i,
            &direction, &number, &transferType, &maxPacketSize, &interval);
        if (kr == KERN_SUCCESS) {
            UInt8 addr = number | (direction << 7);
            if (addr == endpoint_addr) {
                *pipe_ref_out = i;
                return 0;
            }
        }
    }
    return -2;
}

CFRunLoopSourceRef thermal_usb_create_event_source(IOUSBInterfaceInterface **intf) {
    CFRunLoopSourceRef source = NULL;
    kern_return_t kr = (*intf)->CreateInterfaceAsyncEventSource(intf, &source);
    if (kr != KERN_SUCCESS) return NULL;
    return source;
}

int thermal_usb_read_isoch(IOUSBInterfaceInterface **intf,
    uint8_t pipe_ref, void *buf, uint64_t frame_start,
    uint32_t num_frames, IOUSBIsocFrame *frame_list,
    IOAsyncCallback1 callback, void *refcon)
{
    return (*intf)->ReadIsochPipeAsync(intf, pipe_ref,
        buf, frame_start, num_frames, frame_list, callback, refcon);
}

int thermal_usb_get_frame_number(IOUSBInterfaceInterface **intf,
    uint64_t *frame_number, AbsoluteTime *at_time)
{
    return (*intf)->GetBusFrameNumber(intf, frame_number, at_time);
}
```

- [ ] **Step 2: Update `build.rs`**

Replace `src-tauri/build.rs` — remove AVFoundation framework linking:

```rust
fn main() {
    tauri_build::build();

    // Compile the C USB helper that uses IOKit for UVC streaming and control
    cc::Build::new()
        .file("src/usb_helper.c")
        .compile("usb_helper");

    // Link IOKit and CoreFoundation frameworks
    println!("cargo:rustc-link-lib=framework=IOKit");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
}
```

- [ ] **Step 3: Verify C compilation**

```bash
cd src-tauri && cargo build 2>&1 | head -20
```

Expected: C compilation succeeds (Rust code may have errors from other modules — that's OK at this stage).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/usb_helper.c src-tauri/build.rs
git commit -m "feat: rewrite usb_helper.c with explicit-handle IOKit API for streaming"
```

---

## Task 4: USB Stream Module

Create the Rust `usb_stream.rs` module that wraps the C FFI for device lifecycle, streaming, and control transfers.

**Files:**
- Create: `src-tauri/src/usb_stream.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Create `usb_stream.rs` with FFI bindings and `UsbStream` struct**

```rust
//! IOKit USB streaming for PureThermal Y16 video capture.
//!
//! Replaces AVFoundation with direct isochronous USB transfers.
//! Provides both video streaming and control transfer access
//! through a single exclusive device handle.

use std::alloc::{self, Layout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use parking_lot::Mutex;

use crate::camera::types::CameraError;
use crate::uvc_descriptors::{self, UvcStreamConfig};
use crate::uvc_payload::FrameAssembler;

const PT_VID: u16 = 0x1e4e;
const PT_PID: u16 = 0x0100;

// Number of isochronous frames per transfer request
const ISOCH_FRAMES_PER_TRANSFER: u32 = 64;
// Double-buffer: 2 transfer requests in flight
const NUM_TRANSFERS: usize = 2;

// ---------------------------------------------------------------------------
// C FFI declarations
// ---------------------------------------------------------------------------

// IOKit opaque types (pointers to pointers in C, single pointer in Rust FFI)
type IOUSBDeviceInterface = *mut std::ffi::c_void;
type IOUSBInterfaceInterface = *mut std::ffi::c_void;
type CFRunLoopSourceRef = *mut std::ffi::c_void;

/// IOKit IOUSBIsocFrame struct. Must match the C layout exactly.
/// Fields: frStatus (IOReturn/i32), frReqCount (u16), frActCount (u16) = 8 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct IOUSBIsocFrame {
    status: i32,
    req_count: u16,
    act_count: u16,
}
const _: () = assert!(std::mem::size_of::<IOUSBIsocFrame>() == 8);

type IOAsyncCallback1 = extern "C" fn(refcon: *mut std::ffi::c_void, result: i32, arg0: *mut std::ffi::c_void);

extern "C" {
    // io_service_t is mach_port_t = unsigned int = u32 on all current macOS
    fn thermal_usb_find_device(vid: u16, pid: u16, service_out: *mut u32) -> i32;
    fn thermal_usb_open_device(service: u32, dev_out: *mut IOUSBDeviceInterface) -> i32;
    fn thermal_usb_set_configuration(dev: IOUSBDeviceInterface, config: u8) -> i32;
    fn thermal_usb_close_device(dev: IOUSBDeviceInterface);

    fn thermal_usb_find_interface(dev: IOUSBDeviceInterface,
        iface_class: u8, iface_subclass: u8,
        intf_out: *mut IOUSBInterfaceInterface) -> i32;
    fn thermal_usb_open_interface(intf: IOUSBInterfaceInterface) -> i32;
    fn thermal_usb_close_interface(intf: IOUSBInterfaceInterface);
    fn thermal_usb_set_alt_interface(intf: IOUSBInterfaceInterface, alt: u8) -> i32;

    fn thermal_usb_get_config_desc(dev: IOUSBDeviceInterface,
        buf: *mut u8, len: *mut u16) -> i32;

    fn thermal_usb_get_ctrl(dev: IOUSBDeviceInterface,
        unit_id: u8, control_id: u8,
        data: *mut u8, length: u16) -> i32;
    fn thermal_usb_set_ctrl(dev: IOUSBDeviceInterface,
        unit_id: u8, control_id: u8,
        data: *const u8, length: u16) -> i32;

    // Probe/Commit goes through DeviceRequest on device handle.
    // macOS routes class/interface requests to the correct interface based on wIndex.
    fn thermal_usb_vs_probe_set(dev: IOUSBDeviceInterface,
        vs_iface_num: u8, probe_data: *mut u8, len: u16) -> i32;
    fn thermal_usb_vs_probe_get(dev: IOUSBDeviceInterface,
        vs_iface_num: u8, probe_data: *mut u8, len: u16) -> i32;
    fn thermal_usb_vs_commit(dev: IOUSBDeviceInterface,
        vs_iface_num: u8, probe_data: *mut u8, len: u16) -> i32;

    fn thermal_usb_get_pipe_ref(intf: IOUSBInterfaceInterface,
        endpoint_addr: u8, pipe_ref_out: *mut u8) -> i32;
    fn thermal_usb_create_event_source(intf: IOUSBInterfaceInterface) -> CFRunLoopSourceRef;
    fn thermal_usb_read_isoch(intf: IOUSBInterfaceInterface,
        pipe_ref: u8, buf: *mut u8, frame_start: u64,
        num_frames: u32, frame_list: *mut IOUSBIsocFrame,
        callback: IOAsyncCallback1, refcon: *mut std::ffi::c_void) -> i32;
    fn thermal_usb_get_frame_number(intf: IOUSBInterfaceInterface,
        frame_number: *mut u64, at_time: *mut u64) -> i32;

    // CoreFoundation run loop
    fn CFRunLoopGetCurrent() -> *mut std::ffi::c_void;
    fn CFRunLoopAddSource(rl: *mut std::ffi::c_void, source: CFRunLoopSourceRef, mode: *const std::ffi::c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: *mut std::ffi::c_void);
    fn CFRelease(cf: *mut std::ffi::c_void);
}

extern "C" {
    static kCFRunLoopDefaultMode: *const std::ffi::c_void;
}

// ---------------------------------------------------------------------------
// UVC Probe Control structure (26 bytes)
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
struct UvcProbeControl {
    bm_hint: u16,
    b_format_index: u8,
    b_frame_index: u8,
    dw_frame_interval: u32,
    w_key_frame_rate: u16,
    w_p_frame_rate: u16,
    w_comp_quality: u16,
    w_comp_window_size: u16,
    w_delay: u16,
    dw_max_video_frame_size: u32,
    dw_max_payload_transfer_size: u32,
}

// ---------------------------------------------------------------------------
// Per-transfer context for isochronous completion callbacks.
// Each in-flight transfer gets its own refcon so the callback knows
// exactly which buffer/frame_list to process and resubmit.
// ---------------------------------------------------------------------------

struct IsochTransfer {
    /// Raw heap-allocated buffer for isochronous data.
    /// Mutated by IOKit asynchronously — must NOT be a Vec or slice reference.
    buf_ptr: *mut u8,
    buf_size: usize,
    /// Raw heap-allocated IOUSBIsocFrame array.
    frame_list_ptr: *mut IOUSBIsocFrame,
    frame_count: u32,
    /// Back-pointer to shared streaming state.
    shared: Arc<IsochSharedState>,
}

// SAFETY: The raw pointers in IsochTransfer are heap-allocated per-transfer
// and only accessed from the CFRunLoop callback thread.
unsafe impl Send for IsochTransfer {}

impl Drop for IsochTransfer {
    fn drop(&mut self) {
        unsafe {
            let buf_layout = Layout::from_size_align(self.buf_size, 16).unwrap();
            alloc::dealloc(self.buf_ptr, buf_layout);
            let fl_layout = Layout::array::<IOUSBIsocFrame>(self.frame_count as usize).unwrap();
            alloc::dealloc(self.frame_list_ptr as *mut u8, fl_layout);
        }
    }
}

/// State shared between all in-flight transfers and the main UsbStream.
struct IsochSharedState {
    intf: IOUSBInterfaceInterface,
    pipe_ref: u8,
    assembler: Mutex<FrameAssembler>,
    callback: Box<dyn Fn(&[u8], u32, u32) + Send + Sync>,
    width: u32,
    height: u32,
    max_packet_size: u16,
    running: AtomicBool,
    /// Stored by the streaming thread so stop_stream() can call CFRunLoopStop().
    run_loop_ref: Mutex<Option<*mut std::ffi::c_void>>,
}

// SAFETY: IOKit interface pointer is used only from the CFRunLoop thread.
// run_loop_ref is behind Mutex. callback is Send+Sync.
unsafe impl Send for IsochSharedState {}
unsafe impl Sync for IsochSharedState {}

// ---------------------------------------------------------------------------
// UsbStream
// ---------------------------------------------------------------------------

pub struct UsbStream {
    device: IOUSBDeviceInterface,
    vs_intf: IOUSBInterfaceInterface,
    config: UvcStreamConfig,
    stream_thread: Mutex<Option<thread::JoinHandle<()>>>,
    shared_state: Mutex<Option<Arc<IsochSharedState>>>,
}

// SAFETY: IOKit handles are raw pointers. Device handle is used for control
// transfers (synchronized by LeptonController's internal mutex). VS interface
// handle is used by the streaming thread via IsochSharedState. UsbStream's
// Drop impl joins the streaming thread before closing handles.
unsafe impl Send for UsbStream {}
unsafe impl Sync for UsbStream {}

impl UsbStream {
    /// Discover and open the PureThermal device with exclusive access.
    pub fn open() -> Result<Self, CameraError> {
        let mut service: u32 = 0;
        let ret = unsafe { thermal_usb_find_device(PT_VID, PT_PID, &mut service) };
        if ret != 0 {
            return Err(CameraError::DeviceNotFound);
        }

        let mut device: IOUSBDeviceInterface = std::ptr::null_mut();
        let ret = unsafe { thermal_usb_open_device(service, &mut device) };
        if ret != 0 {
            return Err(CameraError::OpenFailed(format!("USBDeviceOpen failed: {ret}")));
        }
        eprintln!("[thermal-v2] USB device opened exclusively");

        // Set configuration 1
        let ret = unsafe { thermal_usb_set_configuration(device, 1) };
        if ret != 0 {
            eprintln!("[thermal-v2] SetConfiguration returned {ret} (may already be set)");
        }

        // Read configuration descriptor
        let mut desc_buf = vec![0u8; 4096];
        let mut desc_len: u16 = 4096;
        let ret = unsafe { thermal_usb_get_config_desc(device, desc_buf.as_mut_ptr(), &mut desc_len) };
        if ret != 0 {
            unsafe { thermal_usb_close_device(device) };
            return Err(CameraError::UvcError(format!("GetConfigDescriptor failed: {ret}")));
        }
        desc_buf.truncate(desc_len as usize);

        // Parse descriptors to find Y16 config
        let config = uvc_descriptors::parse_uvc_config(&desc_buf)
            .map_err(|e| CameraError::UvcError(e))?;
        eprintln!("[thermal-v2] UVC config: format={}, frame={}, {}x{}, interval={}, alt={}, ep=0x{:02X}",
            config.format_index, config.frame_index, config.width, config.height,
            config.frame_interval, config.alt_setting, config.endpoint_addr);

        // Find and open VideoStreaming interface
        let mut vs_intf: IOUSBInterfaceInterface = std::ptr::null_mut();
        let ret = unsafe { thermal_usb_find_interface(device, 0x0E, 0x02, &mut vs_intf) };
        if ret != 0 {
            unsafe { thermal_usb_close_device(device) };
            return Err(CameraError::OpenFailed(format!("VideoStreaming interface not found: {ret}")));
        }
        let ret = unsafe { thermal_usb_open_interface(vs_intf) };
        if ret != 0 {
            unsafe { thermal_usb_close_device(device) };
            return Err(CameraError::OpenFailed(format!("USBInterfaceOpen failed: {ret}")));
        }
        eprintln!("[thermal-v2] VideoStreaming interface opened");

        Ok(Self {
            device,
            vs_intf,
            config,
            stream_thread: Mutex::new(None),
            shared_state: Mutex::new(None),
        })
    }

    /// Start Y16 video streaming with a frame callback.
    /// Callback receives: (y16_data: &[u8], width: u32, height: u32)
    pub fn start_stream<F>(&self, callback: F) -> Result<(), CameraError>
    where
        F: Fn(&[u8], u32, u32) + Send + Sync + 'static,
    {
        // UVC Probe/Commit negotiation
        let mut probe = UvcProbeControl {
            bm_hint: 0x0001,
            b_format_index: self.config.format_index,
            b_frame_index: self.config.frame_index,
            dw_frame_interval: self.config.frame_interval,
            dw_max_video_frame_size: (self.config.width as u32) * (self.config.height as u32) * 2,
            ..Default::default()
        };

        let probe_bytes = unsafe {
            std::slice::from_raw_parts_mut(&mut probe as *mut _ as *mut u8, 26)
        };

        let ret = unsafe { thermal_usb_vs_probe_set(self.device, self.config.vs_interface_num, probe_bytes.as_mut_ptr(), 26) };
        if ret != 0 {
            return Err(CameraError::UvcError(format!("VS_PROBE SET_CUR failed: {ret}")));
        }

        let ret = unsafe { thermal_usb_vs_probe_get(self.device, self.config.vs_interface_num, probe_bytes.as_mut_ptr(), 26) };
        if ret != 0 {
            return Err(CameraError::UvcError(format!("VS_PROBE GET_CUR failed: {ret}")));
        }
        eprintln!("[thermal-v2] Probe negotiated: format={}, frame={}, interval={}",
            probe.b_format_index, probe.b_frame_index, probe.dw_frame_interval);

        let ret = unsafe { thermal_usb_vs_commit(self.device, self.config.vs_interface_num, probe_bytes.as_mut_ptr(), 26) };
        if ret != 0 {
            return Err(CameraError::UvcError(format!("VS_COMMIT SET_CUR failed: {ret}")));
        }

        // Set alt interface to activate isochronous endpoint
        let ret = unsafe { thermal_usb_set_alt_interface(self.vs_intf, self.config.alt_setting) };
        if ret != 0 {
            return Err(CameraError::StreamFailed(format!("SetAlternateInterface({}) failed: {ret}", self.config.alt_setting)));
        }

        // Find pipe ref for the isochronous endpoint
        let mut pipe_ref: u8 = 0;
        let ret = unsafe { thermal_usb_get_pipe_ref(self.vs_intf, self.config.endpoint_addr, &mut pipe_ref) };
        if ret != 0 {
            return Err(CameraError::StreamFailed(format!("Pipe ref not found for endpoint 0x{:02X}: {ret}", self.config.endpoint_addr)));
        }
        eprintln!("[thermal-v2] Pipe ref: {pipe_ref}");

        // Decode wMaxPacketSize (USB 2.0 HS: bits 12:11 = additional transactions)
        let raw_max_pkt = self.config.max_packet_size;
        let pkt_size = (raw_max_pkt & 0x7FF) as usize;
        let mult = ((raw_max_pkt >> 11) & 0x3) as usize + 1;
        let effective_max_pkt = pkt_size * mult;
        eprintln!("[thermal-v2] Max packet: {pkt_size} x {mult} = {effective_max_pkt}");

        let shared = Arc::new(IsochSharedState {
            intf: self.vs_intf,
            pipe_ref,
            assembler: Mutex::new(FrameAssembler::new(self.config.width, self.config.height)),
            callback: Box::new(callback),
            width: self.config.width as u32,
            height: self.config.height as u32,
            max_packet_size: raw_max_pkt,
            running: AtomicBool::new(true),
            run_loop_ref: Mutex::new(None),
        });

        *self.shared_state.lock() = Some(shared.clone());

        let thread = thread::spawn(move || {
            Self::streaming_thread(shared);
        });

        *self.stream_thread.lock() = Some(thread);
        eprintln!("[thermal-v2] Streaming started");
        Ok(())
    }

    fn streaming_thread(shared: Arc<IsochSharedState>) {
        // Create async event source and add to this thread's run loop
        let source = unsafe { thermal_usb_create_event_source(shared.intf) };
        if source.is_null() {
            eprintln!("[thermal-v2] Failed to create async event source");
            return;
        }

        let run_loop = unsafe { CFRunLoopGetCurrent() };
        unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode) };

        // Store the run loop ref so stop_stream() can call CFRunLoopStop()
        *shared.run_loop_ref.lock() = Some(run_loop);

        // Get current bus frame number
        let mut frame_number: u64 = 0;
        let mut at_time: u64 = 0;
        unsafe { thermal_usb_get_frame_number(shared.intf, &mut frame_number, &mut at_time) };
        frame_number += 10; // Start a few frames ahead

        let pkt_size = (shared.max_packet_size & 0x7FF) as usize;
        let mult = ((shared.max_packet_size >> 11) & 0x3) as usize + 1;
        let effective_max_pkt = pkt_size * mult;
        let buf_size = effective_max_pkt * (ISOCH_FRAMES_PER_TRANSFER as usize);

        // Submit initial isochronous transfers (double-buffered).
        // Each transfer owns its buffer and frame_list via raw heap allocation
        // to avoid Rust aliasing violations (IOKit writes asynchronously).
        for _ in 0..NUM_TRANSFERS {
            let buf_layout = Layout::from_size_align(buf_size, 16).unwrap();
            let buf_ptr = unsafe { alloc::alloc_zeroed(buf_layout) };
            let fl_layout = Layout::array::<IOUSBIsocFrame>(ISOCH_FRAMES_PER_TRANSFER as usize).unwrap();
            let frame_list_ptr = unsafe { alloc::alloc_zeroed(fl_layout) } as *mut IOUSBIsocFrame;

            // Initialize frame list: each frame requests max_packet_size bytes
            for i in 0..ISOCH_FRAMES_PER_TRANSFER as usize {
                unsafe {
                    let f = &mut *frame_list_ptr.add(i);
                    f.req_count = shared.max_packet_size;
                    f.act_count = 0;
                    f.status = 0;
                }
            }

            let transfer = Box::new(IsochTransfer {
                buf_ptr,
                buf_size,
                frame_list_ptr,
                frame_count: ISOCH_FRAMES_PER_TRANSFER,
                shared: shared.clone(),
            });

            let transfer_ptr = Box::into_raw(transfer) as *mut std::ffi::c_void;

            let ret = unsafe {
                thermal_usb_read_isoch(
                    shared.intf,
                    shared.pipe_ref,
                    buf_ptr,
                    frame_number,
                    ISOCH_FRAMES_PER_TRANSFER,
                    frame_list_ptr,
                    isoch_completion_callback,
                    transfer_ptr,
                )
            };

            if ret != 0 {
                eprintln!("[thermal-v2] ReadIsochPipeAsync failed: {ret}");
                unsafe { drop(Box::from_raw(transfer_ptr as *mut IsochTransfer)) };
            }

            frame_number += ISOCH_FRAMES_PER_TRANSFER as u64;
        }

        // Run the run loop — blocks until CFRunLoopStop is called
        unsafe { CFRunLoopRun() };
        unsafe { CFRelease(source) };
        eprintln!("[thermal-v2] Streaming thread exited");
    }

    /// Stop streaming and release isochronous resources.
    pub fn stop_stream(&self) -> Result<(), CameraError> {
        if let Some(shared) = self.shared_state.lock().take() {
            shared.running.store(false, Ordering::SeqCst);

            // Stop the run loop (causes CFRunLoopRun to return in the streaming thread)
            if let Some(rl) = shared.run_loop_ref.lock().take() {
                unsafe { CFRunLoopStop(rl) };
            }
        }

        // Join the streaming thread
        if let Some(thread) = self.stream_thread.lock().take() {
            let _ = thread.join();
        }

        // Reset alt interface to 0 (release bandwidth)
        unsafe { thermal_usb_set_alt_interface(self.vs_intf, 0) };

        eprintln!("[thermal-v2] Streaming stopped");
        Ok(())
    }

    /// Send a UVC extension unit GET_CUR control transfer.
    pub fn get_ctrl(&self, unit_id: u8, control_id: u8, data: &mut [u8]) -> Result<usize, CameraError> {
        let ret = unsafe {
            thermal_usb_get_ctrl(self.device, unit_id, control_id, data.as_mut_ptr(), data.len() as u16)
        };
        if ret < 0 {
            return Err(CameraError::LeptonError(format!(
                "GET_CUR failed (unit={unit_id}, ctrl={control_id}): code {ret}"
            )));
        }
        Ok(ret as usize)
    }

    /// Send a UVC extension unit SET_CUR control transfer.
    pub fn set_ctrl(&self, unit_id: u8, control_id: u8, data: &[u8]) -> Result<(), CameraError> {
        let ret = unsafe {
            thermal_usb_set_ctrl(self.device, unit_id, control_id, data.as_ptr(), data.len() as u16)
        };
        if ret < 0 {
            return Err(CameraError::LeptonError(format!(
                "SET_CUR failed (unit={unit_id}, ctrl={control_id}): code {ret}"
            )));
        }
        Ok(())
    }
}

impl Drop for UsbStream {
    fn drop(&mut self) {
        let _ = self.stop_stream();
        unsafe {
            thermal_usb_close_interface(self.vs_intf);
            thermal_usb_close_device(self.device);
        }
        eprintln!("[thermal-v2] USB device closed");
    }
}

// ---------------------------------------------------------------------------
// Isochronous completion callback (called from CFRunLoop thread)
// Each callback invocation corresponds to exactly one IsochTransfer.
// ---------------------------------------------------------------------------

extern "C" fn isoch_completion_callback(
    refcon: *mut std::ffi::c_void,
    result: i32,
    _arg0: *mut std::ffi::c_void,
) {
    // SAFETY: refcon was created by Box::into_raw(IsochTransfer) in streaming_thread
    let mut transfer = unsafe { Box::from_raw(refcon as *mut IsochTransfer) };
    let shared = &transfer.shared;

    if !shared.running.load(Ordering::SeqCst) {
        // Drop the transfer (frees buffer + frame_list)
        return;
    }

    if result != 0 {
        eprintln!("[thermal-v2] Isoch callback error: {result}");
    }

    // Process each microframe's payload data
    let pkt_size = (shared.max_packet_size & 0x7FF) as usize;
    let mult = ((shared.max_packet_size >> 11) & 0x3) as usize + 1;
    let effective_max_pkt = pkt_size * mult;
    let mut offset = 0usize;

    for i in 0..transfer.frame_count as usize {
        let frame = unsafe { &*transfer.frame_list_ptr.add(i) };
        if frame.act_count > 0 && frame.status == 0 {
            let packet = unsafe {
                std::slice::from_raw_parts(transfer.buf_ptr.add(offset), frame.act_count as usize)
            };
            let mut asm = shared.assembler.lock();
            match asm.feed(packet) {
                Ok(Some(y16_data)) => {
                    drop(asm); // release lock before user callback
                    (shared.callback)(&y16_data, shared.width, shared.height);
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("[thermal-v2] Frame assembly error: {e}");
                }
            }
        }
        offset += effective_max_pkt;
    }

    // Reset frame list and resubmit transfer
    for i in 0..transfer.frame_count as usize {
        unsafe {
            let f = &mut *transfer.frame_list_ptr.add(i);
            f.req_count = shared.max_packet_size;
            f.act_count = 0;
            f.status = 0;
        }
    }

    let mut frame_number: u64 = 0;
    let mut at_time: u64 = 0;
    unsafe { thermal_usb_get_frame_number(shared.intf, &mut frame_number, &mut at_time) };

    let transfer_ptr = Box::into_raw(transfer) as *mut std::ffi::c_void;
    let t = unsafe { &*(transfer_ptr as *const IsochTransfer) };

    let ret = unsafe {
        thermal_usb_read_isoch(
            shared.intf,
            shared.pipe_ref,
            t.buf_ptr,
            frame_number + 10,
            t.frame_count,
            t.frame_list_ptr,
            isoch_completion_callback,
            transfer_ptr,
        )
    };

    if ret != 0 {
        eprintln!("[thermal-v2] ReadIsochPipeAsync resubmit failed: {ret}");
        unsafe { drop(Box::from_raw(transfer_ptr as *mut IsochTransfer)) };
    }
}
```

- [ ] **Step 2: Register module in `lib.rs`**

Add `mod usb_stream;` to `src-tauri/src/lib.rs`.

- [ ] **Step 3: Verify compilation**

```bash
cd src-tauri && cargo build 2>&1 | head -30
```

Expected: May have warnings about unused imports; the build should succeed if other modules haven't been updated yet. Fix any compilation errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/usb_stream.rs src-tauri/src/lib.rs
git commit -m "feat: add USB stream module with IOKit isochronous transfers"
```

---

## Task 5: Update `lepton.rs` to Use `UsbStream`

Change `LeptonController` from using `Arc<UsbControl>` to `Arc<UsbStream>`.

**Files:**
- Modify: `src-tauri/src/camera/lepton.rs:9-14,112-126,131-161`

- [ ] **Step 1: Update imports and struct**

In `src-tauri/src/camera/lepton.rs`:

Change line 14:
```rust
// OLD: use crate::usb_control::UsbControl;
use crate::usb_stream::UsbStream;
```

Change lines 112-126 (`LeptonController` struct and `new`):
```rust
pub struct LeptonController {
    usb: Arc<UsbStream>,
    lock: Mutex<()>,
}

unsafe impl Send for LeptonController {}
unsafe impl Sync for LeptonController {}

impl LeptonController {
    pub fn new(usb: Arc<UsbStream>) -> Self {
        Self {
            usb,
            lock: Mutex::new(()),
        }
    }
```

- [ ] **Step 2: Update `get_attribute` and `set_attribute`**

Change `get_attribute` (lines 133-151) — replace `self.usb.get_ctrl(...)` with the new API:

```rust
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
```

Change `set_attribute` (lines 154-162):

```rust
    pub fn set_attribute(&self, command_id: u16, data: &[u16]) -> Result<(), CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);

        let buf: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.usb.set_ctrl(unit_id, control_id, &buf)?;
        Ok(())
    }
```

Note: The method signatures on `UsbStream::get_ctrl`/`set_ctrl` match `UsbControl::get_ctrl`/`set_ctrl` exactly, so the internal code is unchanged — only the type of `self.usb` changes.

- [ ] **Step 3: Run existing tests**

```bash
cd src-tauri && cargo test camera::lepton -- --nocapture
```

Expected: Unit tests PASS (they don't require hardware — they only test `command_to_unit_id` and `command_to_control_id`).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/camera/lepton.rs
git commit -m "refactor: update LeptonController to use UsbStream instead of UsbControl"
```

---

## Task 6: Update `acquisition.rs` to Use `UsbStream`

Replace `AvCamera` with `Arc<UsbStream>`, remove BGRA path.

**Files:**
- Modify: `src-tauri/src/camera/acquisition.rs` (full rewrite)

- [ ] **Step 1: Rewrite `acquisition.rs`**

Replace the entire file:

```rust
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
}

unsafe impl Send for CameraAcquisition {}

impl CameraAcquisition {
    pub fn new(stream: Arc<UsbStream>) -> Self {
        Self {
            stream,
            streaming: Arc::new(AtomicBool::new(false)),
            current_palette: Arc::new(Mutex::new(Palette::IronBlack)),
        }
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.current_palette.lock() = palette;
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }

    pub fn start_stream<F>(&self, on_frame: F) -> Result<(), CameraError>
    where
        F: Fn(FrameResult) + Send + Sync + 'static,
    {
        let palette = self.current_palette.clone();

        self.stream.start_stream(move |y16_data, width, height| {
            let current_palette = *palette.lock();
            let result = processing::process_frame(y16_data, width as usize, height as usize, current_palette);
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
```

- [ ] **Step 2: Verify compilation**

```bash
cd src-tauri && cargo build 2>&1 | head -20
```

Expected: May show errors from `commands/stream.rs` (not yet updated). `acquisition.rs` itself should compile.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/camera/acquisition.rs
git commit -m "refactor: update CameraAcquisition to use UsbStream, remove BGRA path"
```

---

## Task 7: Update Commands and App State

Update `commands/stream.rs` and `lib.rs` to use the unified `UsbStream`.

**Files:**
- Modify: `src-tauri/src/commands/stream.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Rewrite `commands/stream.rs`**

Replace the entire file:

```rust
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::camera::acquisition::CameraAcquisition;
use crate::camera::lepton::LeptonController;
use crate::processing::palettes::Palette;
use crate::usb_stream::UsbStream;
use crate::AppState;

#[derive(Clone, Serialize)]
struct FrameEvent {
    /// Base64-encoded RGBA pixel data
    data: String,
    width: usize,
    height: usize,
    min_val: u16,
    max_val: u16,
}

#[tauri::command]
pub fn connect_camera(state: State<'_, AppState>) -> Result<String, String> {
    eprintln!("[thermal-v2] Connecting via IOKit USB...");

    // Open PureThermal with exclusive access
    let stream = Arc::new(UsbStream::open().map_err(|e| {
        eprintln!("[thermal-v2] USB open failed: {e}");
        e.to_string()
    })?);
    eprintln!("[thermal-v2] USB device opened");

    // Create camera acquisition (for video streaming)
    let cam = CameraAcquisition::new(stream.clone());
    *state.camera.lock() = Some(cam);

    // Create Lepton controller (for SDK commands) using same USB handle
    let lepton = Arc::new(LeptonController::new(stream));
    let part = lepton.get_part_number().unwrap_or_default();
    eprintln!("[thermal-v2] Lepton controller ready, part: {part}");

    *state.lepton.lock() = Some(lepton);
    Ok(part)
}

#[tauri::command]
pub fn start_stream(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    eprintln!("[thermal-v2] start_stream called");
    let cam_guard = state.camera.lock();
    let cam = cam_guard.as_ref().ok_or("Camera not connected")?;

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

#[tauri::command]
pub fn stop_stream(state: State<'_, AppState>) -> Result<(), String> {
    let cam_guard = state.camera.lock();
    if let Some(cam) = cam_guard.as_ref() {
        cam.stop_stream();
    }
    Ok(())
}

#[tauri::command]
pub fn set_palette(state: State<'_, AppState>, palette: String) -> Result<(), String> {
    let cam_guard = state.camera.lock();
    let cam = cam_guard.as_ref().ok_or("Camera not connected")?;
    let p = match palette.as_str() {
        "ironblack" => Palette::IronBlack,
        "rainbow" => Palette::Rainbow,
        "grayscale" => Palette::Grayscale,
        _ => return Err(format!("Unknown palette: {palette}")),
    };
    cam.set_palette(p);
    Ok(())
}
```

Note: `start_stream` and `stop_stream` now use `&self` (not `&mut self`) since `CameraAcquisition` uses interior mutability via `UsbStream`.

- [ ] **Step 2: Update `lib.rs`**

Replace `src-tauri/src/lib.rs`:

```rust
mod camera;
mod commands;
mod processing;
mod usb_stream;
mod uvc_descriptors;
mod uvc_payload;

use camera::acquisition::CameraAcquisition;
use camera::lepton::LeptonController;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub camera: Mutex<Option<CameraAcquisition>>,
    pub lepton: Mutex<Option<Arc<LeptonController>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            camera: Mutex::new(None),
            lepton: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::stream::connect_camera,
            commands::stream::start_stream,
            commands::stream::stop_stream,
            commands::stream::set_palette,
            commands::controls::perform_ffc,
            commands::controls::get_agc_enable,
            commands::controls::set_agc_enable,
            commands::controls::get_agc_policy,
            commands::controls::set_agc_policy,
            commands::controls::get_polarity,
            commands::controls::set_polarity,
            commands::controls::get_gain_mode,
            commands::controls::set_gain_mode,
            commands::controls::get_device_info,
            commands::controls::get_spotmeter_roi,
            commands::controls::set_spotmeter_roi,
            commands::controls::get_spot_temperature,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 3: Verify full build**

```bash
cd src-tauri && cargo build 2>&1
```

Expected: Build succeeds (with possible warnings about dead code in old files not yet deleted).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/stream.rs src-tauri/src/lib.rs
git commit -m "refactor: update commands and app state to use unified UsbStream"
```

---

## Task 8: Cleanup — Remove AVFoundation Code and Dependencies

Delete obsolete files and remove unused dependencies.

**Files:**
- Delete: `src-tauri/src/avfoundation.rs`
- Delete: `src-tauri/src/usb_control.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Delete obsolete files**

```bash
rm src-tauri/src/avfoundation.rs src-tauri/src/usb_control.rs
```

- [ ] **Step 2: Update `Cargo.toml` — remove AVFoundation dependencies**

Replace `src-tauri/Cargo.toml`:

```toml
[package]
name = "thermal-v2"
version = "0.1.0"
description = "FLIR Lepton Thermal Camera Viewer"
authors = ["hmenzagh"]
edition = "2021"

[lib]
name = "thermal_v2_lib"
crate-type = ["lib", "cdylib", "staticlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }
cc = "1"

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-shell = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
base64 = "0.22"
parking_lot = "0.12"
thiserror = "2"
libc = "0.2"
```

Removed: `objc2`, `objc2-foundation`, `objc2-av-foundation`, `objc2-core-media`, `objc2-core-video`, `block2`, `dispatch2`.

- [ ] **Step 3: Verify clean build**

```bash
cd src-tauri && cargo build 2>&1
```

Expected: Build succeeds with no errors.

- [ ] **Step 4: Run all tests**

```bash
cd src-tauri && cargo test -- --nocapture
```

Expected: All unit tests pass (uvc_descriptors, uvc_payload, processing, lepton command mapping).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove AVFoundation code and objc2 dependencies"
```

---

## Task 9: Integration Test with Real Hardware

Manual verification with a connected PureThermal device.

**Files:** None (manual testing)

- [ ] **Step 1: Build and run the app**

```bash
cd /Users/hmenzagh/misc/Thermal_V2 && npm run tauri dev
```

- [ ] **Step 2: Test connection**

Click "Connect Camera" in the UI. Check terminal logs for:
- `USB device opened exclusively`
- `UVC config: format=..., frame=..., 160x120, ...`
- `VideoStreaming interface opened`
- `Lepton controller ready, part: ...`

- [ ] **Step 3: Test streaming**

Verify thermal video appears on the canvas. Check for:
- Frames arriving (logs showing frame processing)
- Correct dimensions (160x120)
- Reasonable frame rate

- [ ] **Step 4: Test palettes**

Switch between IronBlack, Rainbow, and Grayscale. All three should now produce visible different colorizations (unlike BGRA mode where they had no effect).

- [ ] **Step 5: Test AGC and polarity**

Toggle AGC on/off and verify the image contrast changes. Toggle polarity and verify white-hot/black-hot swaps.

- [ ] **Step 6: Test spotmeter and temperature**

Click on the thermal image to set spotmeter position. Verify temperature readings update and show realistic values (~20°C ambient, ~35°C body).

- [ ] **Step 7: Test FFC**

Click FFC button. Verify the shutter click sound and brief image disruption.

- [ ] **Step 8: Test disconnect/reconnect**

Stop streaming, then unplug and replug the PureThermal. Reconnect and verify everything still works.
