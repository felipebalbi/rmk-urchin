#![no_main]
#![no_std]

use panic_probe as _;
use rmk::macros::rmk_central;

#[rmk_central]
mod keybaord_central {}
