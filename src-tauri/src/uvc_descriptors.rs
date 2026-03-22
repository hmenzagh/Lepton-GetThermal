//! UVC descriptor parsing for Y16 (16-bit greyscale) video format detection.
//!
//! Parses raw USB configuration descriptors to locate the Video Streaming
//! interface, Y16 uncompressed format, frame dimensions, and isochronous
//! endpoint needed to set up bulk/isochronous USB transfers.

/// GUID for Y16 (16-bit greyscale) pixel format.
/// Matches the USB Video Class "Y16 " format GUID.
pub const Y16_GUID: [u8; 16] = [
    0x59, 0x31, 0x36, 0x20, // "Y16 "
    0x00, 0x00, 0x10, 0x00,
    0x80, 0x00, 0x00, 0xAA,
    0x00, 0x38, 0x9B, 0x71,
];

/// VS_FORMAT_UNCOMPRESSED descriptor subtype.
pub const VS_FORMAT_UNCOMPRESSED: u8 = 0x04;

/// VS_FRAME_UNCOMPRESSED descriptor subtype.
pub const VS_FRAME_UNCOMPRESSED: u8 = 0x05;

/// Parsed UVC streaming configuration for Y16 video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcStreamConfig {
    /// VideoStreaming interface number.
    pub vs_interface_num: u8,
    /// Format index within the VS interface.
    pub format_index: u8,
    /// Frame index within the format.
    pub frame_index: u8,
    /// Frame width in pixels.
    pub width: u16,
    /// Frame height in pixels.
    pub height: u16,
    /// Default frame interval in 100 ns units.
    pub frame_interval: u32,
    /// Alternate setting that has the isochronous endpoint.
    pub alt_setting: u8,
    /// Endpoint address (IN, isochronous).
    pub endpoint_addr: u8,
    /// Raw wMaxPacketSize from the endpoint descriptor.
    pub max_packet_size: u16,
    /// Bits per pixel from the format descriptor.
    pub bits_per_pixel: u8,
}

impl UvcStreamConfig {
    /// Compute the effective maximum packet size, accounting for
    /// high-bandwidth high-speed isochronous transactions.
    ///
    /// USB 2.0 encodes additional transactions in bits 12:11 of
    /// wMaxPacketSize: effective = (wMaxPacketSize & 0x7FF) * (((wMaxPacketSize >> 11) & 3) + 1)
    pub fn effective_max_packet(&self) -> u16 {
        (self.max_packet_size & 0x7FF) * ((self.max_packet_size >> 11 & 3) + 1)
    }
}

/// Parse a raw USB configuration descriptor blob and extract the Y16
/// UVC streaming configuration.
///
/// Returns `Err` if no Y16 format is found or the descriptor is malformed.
pub fn parse_uvc_config(descriptor: &[u8]) -> Result<UvcStreamConfig, String> {
    // USB descriptor type constants
    const DESC_INTERFACE: u8 = 0x04;
    const DESC_ENDPOINT: u8 = 0x05;
    const DESC_CS_INTERFACE: u8 = 0x24;

    // USB Video class/subclass
    const CC_VIDEO: u8 = 0x0E;
    const SC_VIDEOSTREAMING: u8 = 0x02;

    // Tracking state while walking the descriptor chain
    let mut in_vs_interface = false;
    let mut current_iface_num: u8 = 0;
    let mut current_alt_setting: u8 = 0;

    // Results we're collecting
    let mut vs_interface_num: Option<u8> = None;
    let mut format_index: Option<u8> = None;
    let mut bits_per_pixel: Option<u8> = None;
    let mut frame_index: Option<u8> = None;
    let mut width: Option<u16> = None;
    let mut height: Option<u16> = None;
    let mut frame_interval: Option<u32> = None;
    let mut best_alt_setting: Option<u8> = None;
    let mut best_endpoint_addr: Option<u8> = None;
    let mut best_max_packet_size: u16 = 0;

    let mut pos = 0;
    while pos + 1 < descriptor.len() {
        let b_length = descriptor[pos] as usize;
        if b_length < 2 || pos + b_length > descriptor.len() {
            break;
        }
        let b_descriptor_type = descriptor[pos + 1];

        match b_descriptor_type {
            DESC_INTERFACE if b_length >= 9 => {
                let iface_num = descriptor[pos + 2];
                let alt_setting = descriptor[pos + 3];
                let iface_class = descriptor[pos + 5];
                let iface_subclass = descriptor[pos + 6];

                current_iface_num = iface_num;
                current_alt_setting = alt_setting;
                in_vs_interface = iface_class == CC_VIDEO && iface_subclass == SC_VIDEOSTREAMING;
            }
            DESC_CS_INTERFACE if in_vs_interface && b_length >= 3 => {
                let subtype = descriptor[pos + 2];
                match subtype {
                    VS_FORMAT_UNCOMPRESSED if b_length >= 27 => {
                        // Check GUID at offset +5 (bytes 5..21)
                        let guid_start = pos + 5;
                        let guid_end = guid_start + 16;
                        if guid_end <= descriptor.len()
                            && descriptor[guid_start..guid_end] == Y16_GUID
                        {
                            format_index = Some(descriptor[pos + 3]);
                            bits_per_pixel = Some(descriptor[pos + 21]);
                            vs_interface_num = Some(current_iface_num);
                        }
                    }
                    VS_FRAME_UNCOMPRESSED if b_length >= 26 => {
                        // Only capture if we've already found a Y16 format
                        if format_index.is_some() && frame_index.is_none() {
                            frame_index = Some(descriptor[pos + 3]);
                            width = Some(u16::from_le_bytes([
                                descriptor[pos + 5],
                                descriptor[pos + 6],
                            ]));
                            height = Some(u16::from_le_bytes([
                                descriptor[pos + 7],
                                descriptor[pos + 8],
                            ]));
                            frame_interval = Some(u32::from_le_bytes([
                                descriptor[pos + 21],
                                descriptor[pos + 22],
                                descriptor[pos + 23],
                                descriptor[pos + 24],
                            ]));
                        }
                    }
                    _ => {}
                }
            }
            DESC_ENDPOINT if in_vs_interface && current_alt_setting > 0 && b_length >= 7
                && vs_interface_num.map_or(false, |n| n == current_iface_num) => {
                let ep_addr = descriptor[pos + 2];
                let bm_attributes = descriptor[pos + 3];

                // Check: IN endpoint (bit 7 set) and isochronous (bits 1:0 == 01)
                let is_in = (ep_addr & 0x80) != 0;
                let is_isoch = (bm_attributes & 0x03) == 0x01;

                if is_in && is_isoch {
                    let max_pkt = u16::from_le_bytes([
                        descriptor[pos + 4],
                        descriptor[pos + 5],
                    ]);
                    // Pick the endpoint with the largest max packet size
                    if max_pkt > best_max_packet_size {
                        best_max_packet_size = max_pkt;
                        best_alt_setting = Some(current_alt_setting);
                        best_endpoint_addr = Some(ep_addr);
                    }
                }
            }
            _ => {}
        }

        pos += b_length;
    }

    // Assemble the result, requiring all fields to be present
    let format_idx = format_index.ok_or("Y16 format descriptor not found")?;
    let bpp = bits_per_pixel.ok_or("bits_per_pixel not found")?;
    let vs_iface = vs_interface_num.ok_or("VideoStreaming interface not found")?;
    let f_index = frame_index.ok_or("frame descriptor not found")?;
    let w = width.ok_or("frame width not found")?;
    let h = height.ok_or("frame height not found")?;
    let interval = frame_interval.ok_or("frame interval not found")?;
    let alt = best_alt_setting.ok_or("isochronous endpoint not found")?;
    let ep_addr = best_endpoint_addr.ok_or("isochronous endpoint address not found")?;

    Ok(UvcStreamConfig {
        vs_interface_num: vs_iface,
        format_index: format_idx,
        frame_index: f_index,
        width: w,
        height: h,
        frame_interval: interval,
        alt_setting: alt,
        endpoint_addr: ep_addr,
        max_packet_size: best_max_packet_size,
        bits_per_pixel: bpp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal USB configuration descriptor containing:
    /// - Configuration descriptor header
    /// - VideoControl interface 0 (class 0x0E, subclass 0x01)
    /// - VideoStreaming interface 1, alt 0 (class 0x0E, subclass 0x02)
    /// - CS_INTERFACE: VS_FORMAT_UNCOMPRESSED with Y16 GUID, format index 2
    /// - CS_INTERFACE: VS_FRAME_UNCOMPRESSED 160x120, frame interval 370370
    /// - VideoStreaming interface 1, alt 1 (class 0x0E, subclass 0x02)
    /// - Endpoint 0x81, isochronous IN, max packet size 956
    fn make_test_descriptor() -> Vec<u8> {
        let mut desc = Vec::new();

        // --- Configuration descriptor (9 bytes) ---
        // We'll patch wTotalLength at the end.
        let config_start = desc.len();
        desc.extend_from_slice(&[
            9,    // bLength
            0x02, // bDescriptorType = Configuration
            0, 0, // wTotalLength (placeholder)
            2,    // bNumInterfaces
            1,    // bConfigurationValue
            0,    // iConfiguration
            0x80, // bmAttributes (bus powered)
            250,  // bMaxPower (500 mA)
        ]);

        // --- Interface descriptor: VideoControl (iface 0, alt 0) ---
        desc.extend_from_slice(&[
            9,    // bLength
            0x04, // bDescriptorType = Interface
            0,    // bInterfaceNumber
            0,    // bAlternateSetting
            0,    // bNumEndpoints
            0x0E, // bInterfaceClass = Video
            0x01, // bInterfaceSubClass = VideoControl
            0x00, // bInterfaceProtocol
            0,    // iInterface
        ]);

        // --- Interface descriptor: VideoStreaming (iface 1, alt 0) ---
        desc.extend_from_slice(&[
            9,    // bLength
            0x04, // bDescriptorType = Interface
            1,    // bInterfaceNumber
            0,    // bAlternateSetting
            0,    // bNumEndpoints
            0x0E, // bInterfaceClass = Video
            0x02, // bInterfaceSubClass = VideoStreaming
            0x00, // bInterfaceProtocol
            0,    // iInterface
        ]);

        // --- CS_INTERFACE: VS_FORMAT_UNCOMPRESSED (27 bytes) ---
        let format_desc_len: u8 = 27;
        desc.push(format_desc_len); // bLength
        desc.push(0x24);            // bDescriptorType = CS_INTERFACE
        desc.push(VS_FORMAT_UNCOMPRESSED); // bDescriptorSubtype
        desc.push(2);               // bFormatIndex
        desc.push(1);               // bNumFrameDescriptors
        // guidFormat (16 bytes) — Y16 GUID
        desc.extend_from_slice(&Y16_GUID);
        desc.push(16);              // bBitsPerPixel
        desc.push(1);               // bDefaultFrameIndex
        desc.push(0);               // bAspectRatioX
        desc.push(0);               // bAspectRatioY
        desc.push(0);               // bmInterlaceFlags
        desc.push(0);               // bCopyProtect

        // --- CS_INTERFACE: VS_FRAME_UNCOMPRESSED (30 bytes) ---
        // Minimal frame descriptor: 26 base + 4 bytes for one frame interval = 30
        let frame_desc_len: u8 = 30;
        desc.push(frame_desc_len);  // bLength
        desc.push(0x24);            // bDescriptorType = CS_INTERFACE
        desc.push(VS_FRAME_UNCOMPRESSED); // bDescriptorSubtype
        desc.push(1);               // bFrameIndex
        desc.push(0);               // bmCapabilities
        // wWidth (160) little-endian
        desc.push(160u16 as u8);
        desc.push((160u16 >> 8) as u8);
        // wHeight (120) little-endian
        desc.push(120u16 as u8);
        desc.push((120u16 >> 8) as u8);
        // dwMinBitRate (little-endian, placeholder)
        desc.extend_from_slice(&0u32.to_le_bytes());
        // dwMaxBitRate (little-endian, placeholder)
        desc.extend_from_slice(&0u32.to_le_bytes());
        // dwMaxVideoFrameBufferSize (little-endian, placeholder)
        desc.extend_from_slice(&0u32.to_le_bytes());
        // dwDefaultFrameInterval (370370 = 0x0005A6C2) little-endian
        desc.extend_from_slice(&370370u32.to_le_bytes());
        // bFrameIntervalType = 1 (one discrete interval)
        desc.push(1);
        // dwFrameInterval[0] = 370370
        desc.extend_from_slice(&370370u32.to_le_bytes());

        // --- Interface descriptor: VideoStreaming (iface 1, alt 1) ---
        desc.extend_from_slice(&[
            9,    // bLength
            0x04, // bDescriptorType = Interface
            1,    // bInterfaceNumber
            1,    // bAlternateSetting
            1,    // bNumEndpoints
            0x0E, // bInterfaceClass = Video
            0x02, // bInterfaceSubClass = VideoStreaming
            0x00, // bInterfaceProtocol
            0,    // iInterface
        ]);

        // --- Endpoint descriptor: 0x81, isochronous IN, maxPacketSize 956 ---
        desc.extend_from_slice(&[
            7,    // bLength
            0x05, // bDescriptorType = Endpoint
            0x81, // bEndpointAddress (IN, endpoint 1)
            0x05, // bmAttributes (isochronous, async)
        ]);
        // wMaxPacketSize = 956 (little-endian)
        desc.push(956u16 as u8);
        desc.push((956u16 >> 8) as u8);
        desc.push(1); // bInterval

        // Patch wTotalLength
        let total_len = desc.len() as u16;
        desc[config_start + 2] = total_len as u8;
        desc[config_start + 3] = (total_len >> 8) as u8;

        desc
    }

    #[test]
    fn parse_finds_y16_format() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).expect("should parse Y16 format");
        assert_eq!(config.format_index, 2);
        assert_eq!(config.bits_per_pixel, 16);
    }

    #[test]
    fn parse_finds_frame_dimensions() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).expect("should parse frame dimensions");
        assert_eq!(config.width, 160);
        assert_eq!(config.height, 120);
    }

    #[test]
    fn parse_finds_frame_interval() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).expect("should parse frame interval");
        assert_eq!(config.frame_interval, 370370);
    }

    #[test]
    fn parse_finds_isoch_endpoint() {
        let desc = make_test_descriptor();
        let config = parse_uvc_config(&desc).expect("should parse endpoint");
        assert_eq!(config.vs_interface_num, 1);
        assert_eq!(config.alt_setting, 1);
        assert_eq!(config.endpoint_addr, 0x81);
        assert_eq!(config.max_packet_size, 956);
        assert_eq!(config.effective_max_packet(), 956);
    }

    #[test]
    fn parse_rejects_missing_y16() {
        // Build a descriptor with no Y16 format — just a configuration + VC interface
        let mut desc = Vec::new();
        // Configuration descriptor
        desc.extend_from_slice(&[
            9, 0x02, 0, 0, 1, 1, 0, 0x80, 250,
        ]);
        // VideoControl interface only
        desc.extend_from_slice(&[
            9, 0x04, 0, 0, 0, 0x0E, 0x01, 0x00, 0,
        ]);
        let total = desc.len() as u16;
        desc[2] = total as u8;
        desc[3] = (total >> 8) as u8;

        let result = parse_uvc_config(&desc);
        assert!(result.is_err(), "should reject descriptor without Y16 format");
    }
}
