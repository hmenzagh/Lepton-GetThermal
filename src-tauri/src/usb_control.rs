//! USB control transfer layer via C/IOKit helper.
//!
//! Wraps a minimal C helper (`usb_helper.c`) that uses Apple's IOKit headers
//! directly. This avoids the complexity of constructing IOKit plugin UUIDs
//! from Rust while providing reliable UVC extension unit access.

use crate::camera::types::CameraError;

const PT_VID: u16 = 0x1e4e;
const PT_PID: u16 = 0x0100;

extern "C" {
    fn thermal_usb_open(vid: u16, pid: u16) -> i32;
    fn thermal_usb_get_ctrl(unit_id: u8, control_id: u8, data: *mut u8, length: u16) -> i32;
    fn thermal_usb_set_ctrl(unit_id: u8, control_id: u8, data: *const u8, length: u16) -> i32;
    fn thermal_usb_close();
}

pub struct UsbControl {
    _private: (), // prevent external construction
}

impl UsbControl {
    pub fn connect() -> Result<Self, CameraError> {
        let ret = unsafe { thermal_usb_open(PT_VID, PT_PID) };
        if ret != 0 {
            let msg = match ret {
                -1 => "IOServiceMatching failed",
                -2 => "IOServiceGetMatchingServices failed",
                -3 => "PureThermal device not found",
                -4 => "IOCreatePlugInInterfaceForService failed",
                -5 => "QueryInterface for USB device failed",
                -6 => "USBDeviceOpen failed (device may be in use)",
                _ => "Unknown USB error",
            };
            return Err(CameraError::OpenFailed(format!("{msg} (code: {ret})")));
        }
        eprintln!("[thermal-v2] IOKit USB device opened successfully");
        Ok(Self { _private: () })
    }

    pub fn get_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &mut [u8],
    ) -> Result<usize, CameraError> {
        let ret = unsafe {
            thermal_usb_get_ctrl(unit_id, control_id, data.as_mut_ptr(), data.len() as u16)
        };
        if ret < 0 {
            return Err(CameraError::LeptonError(format!(
                "GET_CUR failed (unit={unit_id}, ctrl={control_id}): code {ret}"
            )));
        }
        Ok(ret as usize)
    }

    pub fn set_ctrl(
        &self,
        unit_id: u8,
        control_id: u8,
        data: &[u8],
    ) -> Result<(), CameraError> {
        let ret = unsafe {
            thermal_usb_set_ctrl(unit_id, control_id, data.as_ptr(), data.len() as u16)
        };
        if ret < 0 {
            return Err(CameraError::LeptonError(format!(
                "SET_CUR failed (unit={unit_id}, ctrl={control_id}): code {ret}"
            )));
        }
        Ok(())
    }
}

impl Drop for UsbControl {
    fn drop(&mut self) {
        unsafe { thermal_usb_close() };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn constants_correct() {
        assert_eq!(super::PT_VID, 0x1e4e);
        assert_eq!(super::PT_PID, 0x0100);
    }
}
