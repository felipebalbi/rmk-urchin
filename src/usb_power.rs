//! Board-local USB supply sensing, independent of USB device enumeration.

pub fn is_usb_power_present() -> bool {
    // Read-only status: do not construct HardwareVbusDetect, change POWER
    // interrupts, or take peripherals already managed by RMK/MPSL.
    embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect()
}
