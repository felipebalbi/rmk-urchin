//! Fixed-size nice!view (LS011B7DH03) framebuffer and asynchronous transport.
//!
//! Wire bytes are sent LSB first, including the one-based physical row address.
//! The panel is 160×68; the keyboard mounts it in 68×160 portrait orientation.
//! Protocol/timing: Sharp LD-29Y03, tables 6-3-1/6-3-2 and section 6-6.

use core::convert::Infallible;
use embedded_graphics::{
    framebuffer::Framebuffer as GraphicsFrameBuffer,
    pixelcolor::{
        BinaryColor,
        raw::{LittleEndian, RawU1},
    },
    prelude::*,
};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, spi::SpiBus};

const ROW_BYTES: usize = 22; // address, 160 pixels, dummy byte
const FRAME_BYTES: usize = 1 + 68 * ROW_BYTES + 1;
pub const ALL_ROWS: u128 = (1 << 68) - 1;
const WRITE: u8 = 0x01;
const VCOM: u8 = 0x02;
const CLEAR: u8 = 0x04;

pub struct FrameBuffer {
    pixels: GraphicsFrameBuffer<BinaryColor, RawU1, LittleEndian, 160, 68, 1360>,
    bytes: [u8; FRAME_BYTES],
}

impl FrameBuffer {
    pub fn new() -> Self {
        let mut pixels = GraphicsFrameBuffer::new();
        let _ = pixels.clear(BinaryColor::On);
        Self {
            pixels,
            bytes: [0; FRAME_BYTES],
        }
    }

    pub fn frame(&mut self, high_polarity: bool) -> &[u8] {
        self.frame_rows(high_polarity, ALL_ROWS)
    }

    /// Serialize selected physical rows (portrait X); ignore bits above row 67.
    /// An empty selection produces HOLD rather than an addressless WRITE.
    pub fn frame_rows(&mut self, high_polarity: bool, rows: u128) -> &[u8] {
        let rows = rows & ALL_ROWS;
        self.bytes[0] = if rows != 0 { WRITE } else { 0 } | if high_polarity { VCOM } else { 0 };
        let mut start = 1;
        for (row, pixels) in self.pixels.data().as_chunks::<20>().0.iter().enumerate() {
            if rows & (1 << row) == 0 {
                continue;
            }
            self.bytes[start] = (row + 1) as u8;
            for (column, byte) in pixels.iter().enumerate() {
                // RawU1 storage is MSB-first even with LittleEndian. The panel
                // expects D0 first, so reverse only pixels, not row addresses.
                self.bytes[start + 1 + column] = byte.reverse_bits();
            }
            self.bytes[start + 21] = 0;
            start += ROW_BYTES;
        }
        self.bytes[start] = 0;
        &self.bytes[..start + 1]
    }
}

impl OriginDimensions for FrameBuffer {
    fn size(&self) -> Size {
        Size::new(68, 160)
    }
}

impl DrawTarget for FrameBuffer {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<BinaryColor>>,
    {
        for Pixel(point, color) in pixels {
            if !(0..68).contains(&point.x) || !(0..160).contains(&point.y) {
                continue;
            }
            let x = 159 - point.y as usize;
            let y = point.x as usize;
            self.pixels.set_pixel(Point::new(x as i32, y as i32), color);
        }
        Ok(())
    }

    fn clear(&mut self, color: BinaryColor) -> Result<(), Self::Error> {
        self.pixels.clear(color)
    }
}

/// Own the active-high select until the entire transaction (including hold
/// time) finishes. Dropping a cancelled SPI future also releases the panel.
struct Selected<'a, CS: OutputPin<Error = Infallible>>(&'a mut CS);

impl<CS: OutputPin<Error = Infallible>> Drop for Selected<'_, CS> {
    fn drop(&mut self) {
        let _ = self.0.set_low();
    }
}

pub struct NiceView<SPI, CS, POWER, DELAY> {
    spi: SPI,
    cs: CS,
    // nice!nano v2 external-supply enable: high while the panel is powered.
    power: POWER,
    delay: DELAY,
    framebuffer: FrameBuffer,
    settled: bool,
}

impl<SPI, CS, POWER, DELAY> NiceView<SPI, CS, POWER, DELAY>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    pub fn new(spi: SPI, mut cs: CS, mut power: POWER, delay: DELAY) -> Self {
        let _ = cs.set_low();
        let _ = power.set_high();
        Self {
            spi,
            cs,
            power,
            delay,
            framebuffer: FrameBuffer::new(),
            settled: false,
        }
    }

    pub fn framebuffer(&mut self) -> &mut FrameBuffer {
        &mut self.framebuffer
    }

    pub async fn settle(&mut self) {
        // DISP is tied to VDD. Allow the switched panel supply to settle before
        // the first clear; the processor creates subscriptions before this wait.
        if !self.settled {
            self.delay.delay_ms(50).await;
            self.settled = true;
        }
    }

    pub async fn power_off(&mut self, high_polarity: bool) -> Result<(), SPI::Error> {
        self.settle().await;
        let result = self
            .command(CLEAR | if high_polarity { VCOM } else { 0 })
            .await;
        let _ = self.cs.set_low();
        // LD-29Y03 §6-2: clear memory, leave serial controls low, then wait T6.
        self.delay.delay_us(30).await;
        let _ = self.power.set_low();
        self.settled = false;
        result
    }

    pub fn power_on(&mut self) {
        let _ = self.cs.set_low();
        let _ = self.power.set_high();
        self.settled = false;
    }

    pub async fn initialize(&mut self, high_polarity: bool) -> Result<(), SPI::Error> {
        self.settle().await;
        self.command(CLEAR | if high_polarity { VCOM } else { 0 })
            .await
    }

    pub async fn flush(&mut self, high_polarity: bool) -> Result<(), SPI::Error> {
        Self::transfer(
            &mut self.spi,
            &mut self.cs,
            &mut self.delay,
            self.framebuffer.frame(high_polarity),
        )
        .await
    }

    /// Write selected physical rows, or maintain VCOM with HOLD if none are selected.
    pub async fn flush_rows(&mut self, high_polarity: bool, rows: u128) -> Result<(), SPI::Error> {
        Self::transfer(
            &mut self.spi,
            &mut self.cs,
            &mut self.delay,
            self.framebuffer.frame_rows(high_polarity, rows),
        )
        .await
    }

    pub async fn maintain(&mut self, high_polarity: bool) -> Result<(), SPI::Error> {
        self.command(if high_polarity { VCOM } else { 0 }).await
    }

    async fn command(&mut self, command: u8) -> Result<(), SPI::Error> {
        Self::transfer(&mut self.spi, &mut self.cs, &mut self.delay, &[command, 0]).await
    }

    async fn transfer(
        spi: &mut SPI,
        cs: &mut CS,
        delay: &mut DELAY,
        bytes: &[u8],
    ) -> Result<(), SPI::Error> {
        let selected = Selected(cs);
        let _ = selected.0.set_high();
        // >=6us setup + 16 clocks + >=2us hold also satisfies the
        // >=22.54us minimum SCS-high duration of a hold-mode command.
        delay.delay_us(6).await;
        spi.write(bytes).await?;
        spi.flush().await?;
        delay.delay_us(2).await;
        drop(selected);
        // A low interval also separates immediately adjacent clear/write frames.
        delay.delay_us(6).await;
        Ok(())
    }
}
