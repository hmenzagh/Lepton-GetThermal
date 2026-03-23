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

    /* Need at least ID320 for USBDeviceOpenSeize support */
    IOUSBDeviceInterface320 **dev = NULL;
    HRESULT hr = (*plugIn)->QueryInterface(plugIn,
        CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID320), (LPVOID *)&dev);
    (*plugIn)->Release(plugIn);
    if (hr != 0 || !dev) return -5;

    /* Use USBDeviceOpenSeize to detach the kernel UVC driver
       (AppleUSBVideoSupport) which claims the VideoStreaming interface.
       Plain USBDeviceOpen would fail with kIOReturnExclusiveAccess. */
    kr = (*dev)->USBDeviceOpenSeize(dev);
    if (kr != KERN_SUCCESS) {
        fprintf(stderr, "[usb_helper] USBDeviceOpenSeize failed: 0x%08X\n", kr);
        (*dev)->Release(dev);
        return -6;
    }

    *dev_out = dev;
    return 0;
}

int thermal_usb_set_configuration(IOUSBDeviceInterface **dev, uint8_t config) {
    /* First set config 0 to force the kernel to release all interface
       claims (AppleUSBVideoSupport), then set the desired config. */
    (*dev)->SetConfiguration(dev, 0);
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

    /* Need ID550 for USBInterfaceOpenSeize (also includes ReadIsochPipeAsync) */
    IOUSBInterfaceInterface550 **intf = NULL;
    HRESULT hr = (*plugIn)->QueryInterface(plugIn,
        CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID550), (LPVOID *)&intf);
    (*plugIn)->Release(plugIn);
    if (hr != 0 || !intf) return -4;

    *intf_out = (IOUSBInterfaceInterface **)intf;
    return 0;
}

int thermal_usb_open_interface(IOUSBInterfaceInterface **intf) {
    /* Use USBInterfaceOpenSeize to detach kernel UVC driver from this interface */
    IOUSBInterfaceInterface550 **intf550 = (IOUSBInterfaceInterface550 **)intf;
    kern_return_t kr = (*intf550)->USBInterfaceOpenSeize(intf550);
    if (kr != KERN_SUCCESS) {
        fprintf(stderr, "[usb_helper] USBInterfaceOpenSeize failed: 0x%08X\n", kr);
    }
    return kr;
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
