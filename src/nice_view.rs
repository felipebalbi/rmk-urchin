//! Fixed-size nice!view (LS011B7DH03) framebuffer and asynchronous transport.
//!
//! Wire bytes are sent LSB first, including the one-based physical row address.
//! The panel is 160×68; the keyboard mounts it in 68×160 portrait orientation.
//! Protocol/timing: Sharp LD-29Y03, tables 6-3-1/6-3-2 and section 6-5.

use core::convert::Infallible;
use embedded_graphics::{
    framebuffer::Framebuffer as GraphicsFrameBuffer,
    pixelcolor::{
        raw::{LittleEndian, RawU1},
        BinaryColor,
    },
    prelude::*,
};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, spi::SpiBus};

const ROW_BYTES: usize = 22; // address, 160 pixels, dummy byte
const FRAME_BYTES: usize = 1 + 68 * ROW_BYTES + 1;
const WRITE: u8 = 0x01;
const VCOM: u8 = 0x02;
const CLEAR: u8 = 0x04;

pub struct FrameBuffer {
    pixels: GraphicsFrameBuffer<BinaryColor, RawU1, LittleEndian, 160, 68, 1360>,
    bytes: [u8; FRAME_BYTES],
}

impl FrameBuffer {
    pub fn new() -> Self {
        let mut bytes = [0; FRAME_BYTES];
        for row in 0..68 {
            let start = 1 + row * ROW_BYTES;
            bytes[start] = (row + 1) as u8;
            bytes[start + 1..start + 21].fill(0xff);
        }
        let mut pixels = GraphicsFrameBuffer::new();
        let _ = pixels.clear(BinaryColor::On);
        Self { pixels, bytes }
    }

    pub fn frame(&mut self, high_polarity: bool) -> &[u8] {
        self.bytes[0] = WRITE | if high_polarity { VCOM } else { 0 };
        for (row, pixels) in self.pixels.data().as_chunks::<20>().0.iter().enumerate() {
            for (column, byte) in pixels.iter().enumerate() {
                // RawU1 storage is MSB-first even with LittleEndian. The panel
                // expects D0 first, so reverse only pixels, not row addresses.
                self.bytes[2 + row * ROW_BYTES + column] = byte.reverse_bits();
            }
        }
        &self.bytes
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
    // Keep the active-high external-supply cutoff owned and driven low.
    _power: POWER,
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
        let _ = power.set_low();
        Self {
            spi,
            cs,
            _power: power,
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
