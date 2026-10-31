pub fn embassy_config() -> embassy_nrf::config::Config {
    let mut config = embassy_nrf::config::Config::default();
    config.lfclk_source = embassy_nrf::config::LfclkSource::ExternalXtal;
    config.dcdc.reg0 = true;
    config.dcdc.reg1 = true;
    config.dcdc.reg0_voltage = Some(embassy_nrf::config::Reg0Voltage::_3V3);
    config
}

pub fn mpsl_config() -> nrf_sdc::mpsl::raw::mpsl_clock_lfclk_cfg_t {
    nrf_sdc::mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: nrf_sdc::mpsl::raw::MPSL_CLOCK_LF_SRC_XTAL as u8,
        rc_ctiv: 0,
        rc_temp_ctiv: 0,
        // Matches Zephyr's crystal-clock accuracy declaration used in the power trial.
        accuracy_ppm: 50,
        skip_wait_lfclk_started: false,
    }
}

pub fn ble_address() -> [u8; 6] {
    let ficr = embassy_nrf::pac::FICR;
    let id = (u64::from(ficr.deviceid(1).read()) << 32) | u64::from(ficr.deviceid(0).read());
    let address = (id | 0x0000_c000_0000_0000).to_le_bytes();
    address[..6].try_into().unwrap()
}
