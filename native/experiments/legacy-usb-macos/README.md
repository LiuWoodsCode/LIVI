# Legacy wired CarPlay on macOS

This experiment investigates an entirely USB transport for phones that reject
`com.apple.carkit.service`. It is separate from the shipping helper and is not a
working CarPlay backend yet.

## Measurements on the attached iOS 12.5.5 phone

- USB configuration 2 exposes audio plus a vendor HID interface (interface 2).
- macOS IOHID accepts output report 14, containing the iAP2 detection marker
  `ff 55 02 00 ee 10` with the HID fragmentation header.
- The phone returns input report 2:
  `02 00 ff 55 02 00 ee 10 00 00`.
- USB configuration 5 exposes USB multiplexing and NCM, but no HID interface.
- Vendor request `0x52`, index 4 (the newer USB mode used by LIVI's Linux
  backend), stalls on this phone.
- All configuration probes restored configuration 5 afterwards.

Thus the phone supports the legacy iAP transport. HID alone does not provide
the IP network needed for entirely wired CarPlay. The next candidate is the
USB role-switch accessory transport, with the Mac exposing iAP2 bulk and NCM
interfaces to the phone.

## MacBook Neo device-mode gate

On the inspected Mac17,5, the T8140 `usb-drd` device-tree node has
`usb-device-disable = <01000000>`. No `IOUSBDeviceController` instance is
published. SIP and AMFI were already disabled on this machine; this experiment
does not change either setting.

`EnableDeviceMode.cpp` is a minimal kernel probe for this gate. It checks for
the exact T8140 compatible string and the expected four-byte flag, saves the
original property, clears it in RAM, and requests matching of the stock
`com.apple.driver.AppleUSBXDCIARM` driver. It does not patch kernel code or
object offsets, terminate the host controller, change boot arguments, or
install itself at boot. It restores the original property on unload. **A
controller that has started is not terminated on unload; a reboot restores the
complete original driver state.** A cached disable flag may prevent activation
even after this probe clears the registry property.

Build only:

```sh
sh native/experiments/legacy-usb-macos/build.sh
```

The output is `/private/tmp/LIVILegacyUSBProbe.kext`. Building does not load it.
Loading experimental kernel code requires explicit approval because it can
disrupt USB or crash the Mac. Inspect the source and save work first.

After an approved load, inspect both the gate and controller publication:

```sh
ioreg -p IODeviceTree -n usb-drd -l -w 0 -r -d 1
ioreg -r -c IOUSBDeviceController -w 0
```

Only if the controller starts should the next stage configure an iAP2 bulk
interface plus built-in NCM, then request the phone's USB role switch.

## References

- [macOS USB device-mode proof of concept](https://github.com/shinyquagsire23/macos_usb_gadget_poc)
- [libusbd device-mode APIs and built-in NCM interfaces](https://github.com/shinyquagsire23/libusbd)
- [CatPlay USB role-switch implementation](https://github.com/catplay-labs/catplay/blob/master/usb/catplay_iap2_usb/src/host/gadget_client.rs)
- [Neo XDCI research and cached-flag limitation](https://github.com/timb-machine-mirrors/dmaynor-apple-vuln-research/blob/main/findings/hw/usb-xdci-device-mode-research.md)
