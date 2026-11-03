// SPDX-License-Identifier: GPL-3.0-or-later
// Experimental runtime probe; this is not part of livi-helperd.

#include <IOKit/IOCatalogue.h>
#include <IOKit/IODeviceTreeSupport.h>
#include <IOKit/IOLib.h>
#include <IOKit/IORegistryEntry.h>
#include <IOKit/IOService.h>
#include <libkern/c++/OSData.h>
#include <libkern/c++/OSSymbol.h>
#include <mach/mach_types.h>

static IORegistryEntry *provider;
static OSData *original;

static void restore_gate()
{
    if (provider && original) {
        provider->setProperty("usb-device-disable", original);
        IOLog("LIVILegacyUSB: restored usb-device-disable\n");
    }
    if (original) original->release();
    if (provider) provider->release();
    original = nullptr;
    provider = nullptr;
}

extern "C" kern_return_t livi_legacy_usb_start(kmod_info_t *, void *)
{
    // Restrict this experiment to the T8140 controller inspected on this Mac.
    // Do not change host drivers, cached object fields, boot args or kernel code.
    auto iter = IORegistryIterator::iterateOver(gIODTPlane, kIORegistryIterateRecursively);
    if (!iter) return KERN_RESOURCE_SHORTAGE;
    while (auto entry = iter->getNextObject()) {
        auto compatible = OSDynamicCast(OSData, entry->getProperty("compatible"));
        if (!compatible || compatible->getLength() != sizeof("usb-drd,t8140") ||
            memcmp(compatible->getBytesNoCopy(), "usb-drd,t8140", sizeof("usb-drd,t8140"))) {
            continue;
        }
        auto gate = OSDynamicCast(OSData, entry->getProperty("usb-device-disable"));
        if (!gate || gate->getLength() != sizeof(uint32_t)) continue;
        uint32_t disabled;
        memcpy(&disabled, gate->getBytesNoCopy(), sizeof(disabled));
        if (disabled != 1) continue;
        provider = entry;
        provider->retain();
        original = gate;
        original->retain();
        break;
    }
    iter->release();
    if (!provider) {
        IOLog("LIVILegacyUSB: disabled T8140 device controller not found\n");
        return KERN_FAILURE;
    }

    const uint32_t enabled = 0;
    auto data = OSData::withBytes(&enabled, sizeof(enabled));
    if (!data || !provider->setProperty("usb-device-disable", data)) {
        if (data) data->release();
        restore_gate();
        return KERN_FAILURE;
    }
    data->release();

    auto bundle = OSSymbol::withCString("com.apple.driver.AppleUSBXDCIARM");
    if (!bundle) {
        restore_gate();
        return KERN_RESOURCE_SHORTAGE;
    }
    const bool queued = gIOCatalogue->startMatching(bundle);
    bundle->release();
    IOLog("LIVILegacyUSB: cleared runtime device gate, XDCI matching queued=%d\n", queued);
    if (!queued) {
        restore_gate();
        return KERN_FAILURE;
    }
    return KERN_SUCCESS;
}

extern "C" kern_return_t livi_legacy_usb_stop(kmod_info_t *, void *)
{
    // Restoring the gate does not terminate a controller that has started.
    // A reboot restores the complete original driver state.
    restore_gate();
    return KERN_SUCCESS;
}

KMOD_EXPLICIT_DECL(dev.fio.livi.legacyusbprobe, "0.1.0", livi_legacy_usb_start, livi_legacy_usb_stop)
__attribute__((visibility("hidden"))) int _kext_apple_cc = __APPLE_CC__;
