/*
 * Minimal IOKit USB control transfer helper for UVC extension units.
 * Called from Rust via FFI. Uses Apple's IOKit headers directly to avoid
 * UUID/CFPlugin issues that arise when constructing IOKit types from Rust.
 */

#include <IOKit/IOKitLib.h>
#include <IOKit/usb/IOUSBLib.h>
#include <IOKit/IOCFPlugIn.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdint.h>

/* Opaque handle to the USB device interface */
static IOUSBDeviceInterface **g_device = NULL;

int thermal_usb_open(uint16_t vid, uint16_t pid) {
    kern_return_t kr;
    io_iterator_t iterator = 0;
    io_service_t service = 0;

    /* Create matching dictionary for USB device */
    CFMutableDictionaryRef matching = IOServiceMatching(kIOUSBDeviceClassName);
    if (!matching) return -1;

    CFNumberRef vidNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &(int32_t){vid});
    CFNumberRef pidNum = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &(int32_t){pid});
    CFDictionarySetValue(matching, CFSTR(kUSBVendorID), vidNum);
    CFDictionarySetValue(matching, CFSTR(kUSBProductID), pidNum);
    CFRelease(vidNum);
    CFRelease(pidNum);

    kr = IOServiceGetMatchingServices(kIOMainPortDefault, matching, &iterator);
    if (kr != KERN_SUCCESS) return -2;

    service = IOIteratorNext(iterator);
    IOObjectRelease(iterator);
    if (!service) return -3; /* Device not found */

    /* Create plugin interface */
    IOCFPlugInInterface **plugIn = NULL;
    SInt32 score = 0;
    kr = IOCreatePlugInInterfaceForService(
        service,
        kIOUSBDeviceUserClientTypeID,
        kIOCFPlugInInterfaceID,
        &plugIn,
        &score
    );
    IOObjectRelease(service);
    if (kr != KERN_SUCCESS || !plugIn) return -4;

    /* Query for the USB device interface */
    IOUSBDeviceInterface **dev = NULL;
    HRESULT hr = (*plugIn)->QueryInterface(
        plugIn,
        CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID),
        (LPVOID *)&dev
    );
    (*plugIn)->Release(plugIn);
    if (hr != 0 || !dev) return -5;

    /* NOTE: We intentionally do NOT call USBDeviceOpen() here.
     * USBDeviceOpen() takes exclusive access to the device, which blocks
     * AVFoundation from receiving video frames. DeviceRequest() works
     * without opening the device on macOS for control transfers. */
    g_device = dev;
    return 0;
}

int thermal_usb_get_ctrl(uint8_t unit_id, uint8_t control_id,
                          uint8_t *data, uint16_t length) {
    if (!g_device) return -1;

    IOUSBDevRequest req;
    req.bmRequestType = USBmakebmRequestType(kUSBIn, kUSBClass, kUSBInterface);
    req.bRequest = 0x81; /* GET_CUR */
    req.wValue = (uint16_t)control_id << 8;
    req.wIndex = (uint16_t)unit_id << 8; /* interface 0 */
    req.wLength = length;
    req.pData = data;
    req.wLenDone = 0;

    kern_return_t kr = (*g_device)->DeviceRequest(g_device, &req);
    if (kr != KERN_SUCCESS) return -(int)kr;

    return (int)req.wLenDone;
}

int thermal_usb_set_ctrl(uint8_t unit_id, uint8_t control_id,
                          const uint8_t *data, uint16_t length) {
    if (!g_device) return -1;

    IOUSBDevRequest req;
    req.bmRequestType = USBmakebmRequestType(kUSBOut, kUSBClass, kUSBInterface);
    req.bRequest = 0x01; /* SET_CUR */
    req.wValue = (uint16_t)control_id << 8;
    req.wIndex = (uint16_t)unit_id << 8; /* interface 0 */
    req.wLength = length;
    req.pData = (void *)data;
    req.wLenDone = 0;

    kern_return_t kr = (*g_device)->DeviceRequest(g_device, &req);
    if (kr != KERN_SUCCESS) return -(int)kr;

    return 0;
}

void thermal_usb_close(void) {
    if (g_device) {
        /* No USBDeviceClose needed since we never called USBDeviceOpen */
        (*g_device)->Release(g_device);
        g_device = NULL;
    }
}
