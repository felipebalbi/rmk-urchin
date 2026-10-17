#![no_main]
#![no_std]

use panic_probe as _;
use rmk::macros::rmk_peripheral;

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {}
