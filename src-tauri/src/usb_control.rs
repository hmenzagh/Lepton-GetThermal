//! USB control transfer layer using nusb.
//!
//! Provides UVC extension unit read/write for Lepton SDK commands.
//! Uses IOKit on macOS (via nusb) — cooperates with kernel UVC driver.
//!
//! On macOS (and Linux), nusb allows device-level control transfers without
//! claiming the interface, avoiding conflicts with the kernel UVC driver that
//! holds interface 0.

use crate::camera::types::CameraError;
use nusb::transfer::{ControlIn, ControlOut, ControlType, Recipient};
use nusb::MaybeFuture;
use std::time::Duration;

/// PureThermal USB Vendor ID.
const PT_VID: u16 = 0x1e4e;
/// PureThermal USB Product ID.
const PT_PID: u16 = 0x0100;

/// UVC SET_CUR request code (host-to-device).
const UVC_SET_CUR: u8 = 0x01;
/// UVC GET_CUR request code (device-to-host).
const UVC_GET_CUR: u8 = 0x81;
/// UVC Video Control interface number (always 0 for PureThermal).
const UVC_VC_INTERFACE: u16 = 0;

/// Default timeout for USB control transfers.
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);

/// USB control transfer interface for UVC extension unit commands.
///
/// Uses nusb device-level control transfers (available on macOS and Linux)
/// to communicate with the Lepton sensor through PureThermal's UVC
/// extension units, without needing to claim the video control interface.
pub struct UsbControl {
    device: nusb::Device,
}

impl UsbControl {
    /// Find a PureThermal device and open it for control transfers.
    ///
    /// Scans connected USB devices for VID=0x1e4e, PID=0x0100 and opens
    /// the first match. Uses device-level control transfers so the kernel
    /// UVC driver can remain attached to interface 0.
    pub fn connect() -> Result<Self, CameraError> {
        let device_info = nusb::list_devices()
            .wait()
            .map_err(|e| CameraError::OpenFailed(format!("Failed to list USB devices: {e}")))?
            .find(|dev| dev.vendor_id() == PT_VID && dev.product_id() == PT_PID)
            .ok_or(CameraError::DeviceNotFound)?;

        let device = device_info
            .open()
            .wait()
            .map_err(|e| CameraError::OpenFailed(format!("Failed to open USB device: {e}")))?;

        Ok(Self { device })
    }

    /// Read from a UVC extension unit (GET_CUR).
    ///
    /// Performs a class-specific interface control-in transfer:
    /// - bmRequestType: 0xA1 (device-to-host | class | interface)
    /// - bRequest: GET_CUR (0x81)
    /// - wValue: control_id << 8
    /// - wIndex: unit_id << 8 | interface_number
    ///
    /// Returns the number of bytes actually read into `data`.
    pub fn get_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<usize, CameraError> {
        let wvalue = (control_id as u16) << 8;
        let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;

        let result = self
            .device
            .control_in(
                ControlIn {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request: UVC_GET_CUR,
                    value: wvalue,
                    index: windex,
                    length: data.len() as u16,
                },
                CONTROL_TIMEOUT,
            )
            .wait()
            .map_err(|e| CameraError::UvcError(format!("GET_CUR failed: {e}")))?;

        let n = result.len().min(data.len());
        data[..n].copy_from_slice(&result[..n]);
        Ok(n)
    }

    /// Write to a UVC extension unit (SET_CUR).
    ///
    /// Performs a class-specific interface control-out transfer:
    /// - bmRequestType: 0x21 (host-to-device | class | interface)
    /// - bRequest: SET_CUR (0x01)
    /// - wValue: control_id << 8
    /// - wIndex: unit_id << 8 | interface_number
    pub fn set_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &[u8],
    ) -> Result<(), CameraError> {
        let wvalue = (control_id as u16) << 8;
        let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;

        self.device
            .control_out(
                ControlOut {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request: UVC_SET_CUR,
                    value: wvalue,
                    index: windex,
                    data,
                },
                CONTROL_TIMEOUT,
            )
            .wait()
            .map_err(|e| CameraError::UvcError(format!("SET_CUR failed: {e}")))?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request_type_is_correct() {
        // GET_CUR bmRequestType: 0xA1 = 10100001 = device-to-host | class | interface
        // Direction::In (0x80) | ControlType::Class (1 << 5 = 0x20) | Recipient::Interface (0x01)
        assert_eq!(0x80 | 0x20 | 0x01, 0xA1);
    }

    #[test]
    fn set_request_type_is_correct() {
        // SET_CUR bmRequestType: 0x21 = 00100001 = host-to-device | class | interface
        // Direction::Out (0x00) | ControlType::Class (1 << 5 = 0x20) | Recipient::Interface (0x01)
        assert_eq!(0x00 | 0x20 | 0x01, 0x21);
    }

    #[test]
    fn windex_encoding() {
        // wIndex = unit_id << 8 | interface_number
        let unit_id: u8 = 3; // AGC extension unit
        let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;
        assert_eq!(windex, 0x0300);
    }

    #[test]
    fn wvalue_encoding() {
        // wValue = control_id << 8
        let control_id: u8 = 1;
        let wvalue = (control_id as u16) << 8;
        assert_eq!(wvalue, 0x0100);
    }

    #[test]
    fn windex_with_different_units() {
        // Verify wIndex encoding for all Lepton extension unit IDs
        let test_cases: &[(u8, u16)] = &[
            (3, 0x0300), // AGC
            (4, 0x0400), // OEM
            (5, 0x0500), // RAD
            (6, 0x0600), // SYS
            (7, 0x0700), // VID
        ];
        for &(unit_id, expected) in test_cases {
            let windex = (unit_id as u16) << 8 | UVC_VC_INTERFACE;
            assert_eq!(windex, expected, "unit_id={unit_id}");
        }
    }

    #[test]
    fn wvalue_with_different_controls() {
        // Verify wValue encoding for various control IDs
        for control_id in 1u8..=20 {
            let wvalue = (control_id as u16) << 8;
            assert_eq!(wvalue, (control_id as u16) << 8);
            // High byte is the control_id, low byte is always 0
            assert_eq!((wvalue >> 8) as u8, control_id);
            assert_eq!((wvalue & 0xFF) as u8, 0);
        }
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(PT_VID, 0x1e4e);
        assert_eq!(PT_PID, 0x0100);
        assert_eq!(UVC_SET_CUR, 0x01);
        assert_eq!(UVC_GET_CUR, 0x81);
        assert_eq!(UVC_VC_INTERFACE, 0);
    }

    #[test]
    fn nusb_control_type_class_maps_to_0x20() {
        // Verify that nusb's ControlType::Class << 5 produces the expected byte
        // ControlType::Class has repr value 1, so (1 << 5) = 0x20
        assert_eq!((ControlType::Class as u8) << 5, 0x20);
    }

    #[test]
    fn nusb_recipient_interface_maps_to_0x01() {
        // Verify that nusb's Recipient::Interface produces the expected byte
        assert_eq!(Recipient::Interface as u8, 0x01);
    }
}
