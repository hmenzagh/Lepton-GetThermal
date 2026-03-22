//! UVC isochronous payload header parsing and Y16 frame reassembly.
//!
//! Parses UVC payload headers from isochronous USB transfer packets and
//! reassembles them into complete Y16 video frames. Fully testable without
//! hardware.

/// Bit-field Header (BFH) flag: Frame ID toggle.
pub const BFH_FID: u8 = 0x01;
/// Bit-field Header (BFH) flag: End of Frame.
pub const BFH_EOF: u8 = 0x02;
/// Bit-field Header (BFH) flag: Presentation Time Stamp present.
pub const BFH_PTS: u8 = 0x04;
/// Bit-field Header (BFH) flag: Source Clock Reference present.
pub const BFH_SCR: u8 = 0x08;
/// Bit-field Header (BFH) flag: Still Image.
pub const BFH_STI: u8 = 0x20;
/// Bit-field Header (BFH) flag: Error.
pub const BFH_ERR: u8 = 0x40;
/// Bit-field Header (BFH) flag: End of Header.
pub const BFH_EOH: u8 = 0x80;

/// Parsed UVC payload header from an isochronous transfer packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UvcPayloadHeader {
    /// Length of the header in bytes.
    pub header_len: u8,
    /// Frame ID bit — toggles between consecutive frames.
    pub fid: bool,
    /// End of frame flag.
    pub eof: bool,
    /// Error flag — indicates the device detected an error for this frame.
    pub err: bool,
    /// Presentation Time Stamp (if present).
    pub pts: Option<u32>,
    /// Source Clock Reference (if present).
    pub scr: Option<[u8; 6]>,
}

/// Parse a UVC payload header from raw isochronous packet data.
///
/// Returns the parsed header and the byte offset where payload data begins.
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
        let val = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        offset += 4;
        Some(val)
    } else {
        None
    };

    let scr = if has_scr && offset + 6 <= header_len as usize {
        let mut s = [0u8; 6];
        s.copy_from_slice(&data[offset..offset + 6]);
        offset += 6;
        Some(s)
    } else {
        None
    };

    Ok((
        UvcPayloadHeader {
            header_len,
            fid,
            eof,
            err,
            pts,
            scr,
        },
        header_len as usize,
    ))
}

/// Assembles complete Y16 frames from a sequence of UVC isochronous packets.
pub struct FrameAssembler {
    /// Accumulated payload data for the current frame.
    buffer: Vec<u8>,
    /// Expected frame size in bytes (width * height * 2 for Y16).
    expected_size: usize,
    /// Last observed FID bit, used to detect frame boundaries.
    last_fid: Option<bool>,
}

impl FrameAssembler {
    /// Create a new assembler for frames of the given dimensions.
    ///
    /// `expected_size` is computed as `width * height * 2` (Y16 = 2 bytes/pixel).
    pub fn new(width: u16, height: u16) -> Self {
        let expected_size = width as usize * height as usize * 2;
        Self {
            buffer: Vec::with_capacity(expected_size),
            expected_size,
            last_fid: None,
        }
    }

    /// Feed a raw isochronous packet into the assembler.
    ///
    /// Returns `Ok(Some(frame))` when a complete frame has been assembled,
    /// `Ok(None)` when more packets are needed, or `Err` if the packet
    /// header is malformed.
    pub fn feed(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let (header, data_offset) = parse_payload_header(packet)?;

        if header.err {
            self.buffer.clear();
            self.last_fid = Some(header.fid);
            return Ok(None);
        }

        // FID toggle means a new frame started — discard any incomplete data
        if let Some(last) = self.last_fid {
            if last != header.fid && !self.buffer.is_empty() {
                self.buffer.clear();
            }
        }
        self.last_fid = Some(header.fid);

        // Append payload data (everything after the header)
        if data_offset < packet.len() {
            self.buffer.extend_from_slice(&packet[data_offset..]);
        }

        if header.eof {
            if self.buffer.len() == self.expected_size {
                let frame = std::mem::take(&mut self.buffer);
                self.buffer.reserve(self.expected_size);
                return Ok(Some(frame));
            } else {
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

    /// Reset the assembler state, discarding any partial frame data.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.last_fid = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_header() {
        // 2-byte header with FID set, no PTS/SCR
        let data = [2u8, BFH_FID | BFH_EOH];
        let (header, offset) = parse_payload_header(&data).unwrap();
        assert_eq!(header.header_len, 2);
        assert!(header.fid);
        assert!(!header.eof);
        assert!(!header.err);
        assert_eq!(header.pts, None);
        assert_eq!(header.scr, None);
        assert_eq!(offset, 2);
    }

    #[test]
    fn parse_header_with_pts() {
        // 12-byte header with PTS=42 and SCR
        let mut data = vec![12u8, BFH_FID | BFH_PTS | BFH_SCR | BFH_EOH];
        // PTS (4 bytes LE)
        data.extend_from_slice(&42u32.to_le_bytes());
        // SCR (6 bytes)
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let (header, offset) = parse_payload_header(&data).unwrap();
        assert_eq!(header.header_len, 12);
        assert!(header.fid);
        assert_eq!(header.pts, Some(42));
        assert_eq!(header.scr, Some([1, 2, 3, 4, 5, 6]));
        assert_eq!(offset, 12);
    }

    #[test]
    fn parse_header_error_flag() {
        let data = [2u8, BFH_ERR | BFH_EOH];
        let (header, _) = parse_payload_header(&data).unwrap();
        assert!(header.err);
    }

    #[test]
    fn parse_empty_data_fails() {
        assert!(parse_payload_header(&[]).is_err());
        assert!(parse_payload_header(&[1]).is_err());
    }

    #[test]
    fn assemble_single_packet_frame() {
        // 2x2 Y16 frame (8 bytes) in a single packet with EOF
        let mut assembler = FrameAssembler::new(2, 2);
        let mut packet = vec![2u8, BFH_FID | BFH_EOF | BFH_EOH];
        packet.extend_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]);
        let result = assembler.feed(&packet).unwrap();
        assert!(result.is_some());
        let frame = result.unwrap();
        assert_eq!(frame.len(), 8);
        assert_eq!(frame, &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]);
    }

    #[test]
    fn assemble_multi_packet_frame() {
        // 2x2 Y16 frame split across 2 packets
        let mut assembler = FrameAssembler::new(2, 2);

        // First packet: first 4 bytes of payload, no EOF
        let mut pkt1 = vec![2u8, BFH_FID | BFH_EOH];
        pkt1.extend_from_slice(&[0x10, 0x20, 0x30, 0x40]);
        assert!(assembler.feed(&pkt1).unwrap().is_none());

        // Second packet: last 4 bytes of payload, with EOF
        let mut pkt2 = vec![2u8, BFH_FID | BFH_EOF | BFH_EOH];
        pkt2.extend_from_slice(&[0x50, 0x60, 0x70, 0x80]);
        let result = assembler.feed(&pkt2).unwrap();
        assert!(result.is_some());
        let frame = result.unwrap();
        assert_eq!(frame, &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]);
    }

    #[test]
    fn assemble_discards_on_error() {
        // ERR bit discards buffer, next frame works
        let mut assembler = FrameAssembler::new(2, 2);

        // Start a frame
        let mut pkt1 = vec![2u8, BFH_FID | BFH_EOH];
        pkt1.extend_from_slice(&[0x10, 0x20, 0x30, 0x40]);
        assert!(assembler.feed(&pkt1).unwrap().is_none());

        // Error packet — should discard buffer
        let err_pkt = vec![2u8, BFH_FID | BFH_ERR | BFH_EOH];
        assert!(assembler.feed(&err_pkt).unwrap().is_none());

        // New frame should still work
        let mut pkt2 = vec![2u8, BFH_FID | BFH_EOF | BFH_EOH];
        pkt2.extend_from_slice(&[0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01, 0x02]);
        let result = assembler.feed(&pkt2).unwrap();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            &[0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01, 0x02]
        );
    }

    #[test]
    fn assemble_fid_toggle_starts_new_frame() {
        // FID toggle discards incomplete frame, starts new one
        let mut assembler = FrameAssembler::new(2, 2);

        // Start frame with FID=1
        let mut pkt1 = vec![2u8, BFH_FID | BFH_EOH];
        pkt1.extend_from_slice(&[0x10, 0x20, 0x30, 0x40]);
        assert!(assembler.feed(&pkt1).unwrap().is_none());

        // FID toggles to 0 — incomplete frame discarded, new frame starts
        let mut pkt2 = vec![2u8, BFH_EOH]; // FID=0
        pkt2.extend_from_slice(&[0xA0, 0xB0, 0xC0, 0xD0]);
        assert!(assembler.feed(&pkt2).unwrap().is_none());

        // Complete the new frame with FID=0
        let mut pkt3 = vec![2u8, BFH_EOF | BFH_EOH]; // FID=0, EOF
        pkt3.extend_from_slice(&[0xE0, 0xF0, 0x01, 0x02]);
        let result = assembler.feed(&pkt3).unwrap();
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            &[0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01, 0x02]
        );
    }

    #[test]
    fn assemble_drops_wrong_size_frame() {
        // EOF with wrong size drops the frame
        let mut assembler = FrameAssembler::new(2, 2); // expects 8 bytes

        // Send only 4 bytes then EOF
        let mut pkt = vec![2u8, BFH_FID | BFH_EOF | BFH_EOH];
        pkt.extend_from_slice(&[0x10, 0x20, 0x30, 0x40]);
        let result = assembler.feed(&pkt).unwrap();
        assert!(result.is_none());
    }
}
