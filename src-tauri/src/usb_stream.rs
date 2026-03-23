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
        eprintln!("[lepton-getthermal] USB device opened exclusively");

        // Set configuration 1
        let ret = unsafe { thermal_usb_set_configuration(device, 1) };
        if ret != 0 {
            eprintln!("[lepton-getthermal] SetConfiguration returned {ret} (may already be set)");
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
        eprintln!("[lepton-getthermal] UVC config: format={}, frame={}, {}x{}, interval={}, alt={}, ep=0x{:02X}",
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
        eprintln!("[lepton-getthermal] VideoStreaming interface opened");

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
        let neg_format = probe.b_format_index;
        let neg_frame = probe.b_frame_index;
        let neg_interval = { probe.dw_frame_interval };
        eprintln!("[lepton-getthermal] Probe negotiated: format={}, frame={}, interval={}",
            neg_format, neg_frame, neg_interval);

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
        eprintln!("[lepton-getthermal] Pipe ref: {pipe_ref}");

        // Decode wMaxPacketSize (USB 2.0 HS: bits 12:11 = additional transactions)
        let raw_max_pkt = self.config.max_packet_size;
        let pkt_size = (raw_max_pkt & 0x7FF) as usize;
        let mult = ((raw_max_pkt >> 11) & 0x3) as usize + 1;
        let effective_max_pkt = pkt_size * mult;
        eprintln!("[lepton-getthermal] Max packet: {pkt_size} x {mult} = {effective_max_pkt}");

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
        eprintln!("[lepton-getthermal] Streaming started");
        Ok(())
    }

    fn streaming_thread(shared: Arc<IsochSharedState>) {
        // Create async event source and add to this thread's run loop
        let source = unsafe { thermal_usb_create_event_source(shared.intf) };
        if source.is_null() {
            eprintln!("[lepton-getthermal] Failed to create async event source");
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
                eprintln!("[lepton-getthermal] ReadIsochPipeAsync failed: {ret}");
                unsafe { drop(Box::from_raw(transfer_ptr as *mut IsochTransfer)) };
            }

            frame_number += ISOCH_FRAMES_PER_TRANSFER as u64;
        }

        // Run the run loop — blocks until CFRunLoopStop is called
        unsafe { CFRunLoopRun() };
        unsafe { CFRelease(source) };
        eprintln!("[lepton-getthermal] Streaming thread exited");
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

        eprintln!("[lepton-getthermal] Streaming stopped");
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
        eprintln!("[lepton-getthermal] USB device closed");
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
    let transfer = unsafe { Box::from_raw(refcon as *mut IsochTransfer) };
    let shared = transfer.shared.clone();

    if !shared.running.load(Ordering::SeqCst) {
        // Drop the transfer (frees buffer + frame_list)
        return;
    }

    if result != 0 {
        eprintln!("[lepton-getthermal] Isoch callback error: {result}");
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
                    eprintln!("[lepton-getthermal] Frame assembly error: {e}");
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
        eprintln!("[lepton-getthermal] ReadIsochPipeAsync resubmit failed: {ret}");
        unsafe { drop(Box::from_raw(transfer_ptr as *mut IsochTransfer)) };
    }
}
