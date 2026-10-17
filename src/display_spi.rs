//! Cancellation boundary for embassy-nrf 0.11's otherwise unguarded SPIM DMA.

use core::{
    future::Future,
    pin::pin,
    sync::atomic::{compiler_fence, Ordering},
};
use embedded_hal_async::spi::SpiBus;

/// Register operations needed to stop one exclusively owned DMA peripheral.
pub(crate) trait DmaControl {
    fn disable_end_interrupt(&mut self);
    fn ended(&self) -> bool;
    fn clear_stopped(&mut self);
    fn stop(&mut self);
    fn stopped(&mut self) -> bool;
    fn reset_after_abort(&mut self);
}

pub(crate) struct CancelSafe<SPI, CONTROL> {
    spi: SPI,
    control: CONTROL,
}

impl<SPI, CONTROL> CancelSafe<SPI, CONTROL> {
    /// # Safety
    /// `control` must quiesce the same exclusively owned hardware as `spi`.
    /// Ready SPI operations must no longer access their buffers. No other actor
    /// (including interrupt handlers) may start transactions. The inner future
    /// must keep any internal DMA buffers alive until it is dropped.
    pub(crate) unsafe fn new(spi: SPI, control: CONTROL) -> Self {
        Self { spi, control }
    }
}

struct Cancellation<'a, C: DmaControl> {
    control: &'a mut C,
    armed: bool,
}

impl<C: DmaControl> Drop for Cancellation<'_, C> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.control.disable_end_interrupt();
        // END also proves both EasyDMA channels have finished. A completed but
        // unpolled future needs no STOP; otherwise wait for the current byte to
        // finish and STOPPED before releasing either caller or inner buffers.
        if !self.control.ended() {
            self.control.clear_stopped();
            self.control.stop();
            while !self.control.stopped() {
                core::hint::spin_loop();
            }
        }
        compiler_fence(Ordering::SeqCst);
        self.control.reset_after_abort();
    }
}

async fn guarded<F: Future, C: DmaControl>(future: F, control: &mut C) -> F::Output {
    // This pinned owning storage MUST precede the guard. The awaited Pin below
    // only borrows it. On cancellation the guard drops first, quiescing DMA
    // before the inner future (including HAL copy buffers) can be destroyed.
    let future = pin!(future);
    let mut cancellation = Cancellation {
        control,
        armed: true,
    };
    let result = future.await;
    cancellation.armed = false;
    result
}

impl<SPI: embedded_hal::spi::ErrorType, C> embedded_hal::spi::ErrorType for CancelSafe<SPI, C> {
    type Error = SPI::Error;
}

impl<SPI: SpiBus<u8>, C: DmaControl> SpiBus<u8> for CancelSafe<SPI, C> {
    async fn read(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        guarded(self.spi.read(bytes), &mut self.control).await
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        guarded(self.spi.write(bytes), &mut self.control).await
    }
    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        guarded(self.spi.transfer(read, write), &mut self.control).await
    }
    async fn transfer_in_place(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        guarded(self.spi.transfer_in_place(bytes), &mut self.control).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        guarded(self.spi.flush(), &mut self.control).await
    }
}

#[cfg(target_os = "none")]
mod nrf {
    use super::*;
    use embassy_nrf::{
        interrupt::{self, typelevel::Interrupt},
        peripherals, spim, Peri,
    };

    struct Spi2Control;

    impl DmaControl for Spi2Control {
        fn disable_end_interrupt(&mut self) {
            embassy_nrf::pac::SPIM2
                .intenclr()
                .write(|w| w.set_end(true));
        }
        fn ended(&self) -> bool {
            embassy_nrf::pac::SPIM2.events_end().read() != 0
        }
        fn clear_stopped(&mut self) {
            embassy_nrf::pac::SPIM2.events_stopped().write_value(0);
        }
        fn stop(&mut self) {
            embassy_nrf::pac::SPIM2.tasks_stop().write_value(1);
        }
        fn stopped(&mut self) -> bool {
            embassy_nrf::pac::SPIM2.events_stopped().read() != 0
        }
        fn reset_after_abort(&mut self) {
            let r = embassy_nrf::pac::SPIM2;
            // HAL's nRF52840 ISR only wakes and disables END interrupts; it
            // never starts DMA. With END masked and DMA quiescent, retire all
            // stale events/pointers before the next HAL write arms END again.
            r.events_end().write_value(0);
            r.events_endtx().write_value(0);
            r.events_endrx().write_value(0);
            r.events_started().write_value(0);
            r.events_stopped().write_value(0);
            r.dma().tx().maxcnt().write(|w| w.set_maxcnt(0));
            r.dma().rx().maxcnt().write(|w| w.set_maxcnt(0));
            r.dma().tx().ptr().write_value(0);
            r.dma().rx().ptr().write_value(0);
            interrupt::typelevel::SPI2::unpend();
        }
    }

    /// Owns SPI2 through the HAL for its entire lifetime. Avoids the nRF52840
    /// SPIM3-specific anomaly 198 without reserving a RAM bank. No raw peripheral or
    /// inner HAL handle escapes. Normal transfers remain interrupt-driven;
    /// only cancellation synchronously waits for STOPPED (at most the current
    /// byte in normal hardware operation), without globally masking interrupts.
    ///
    /// Nordic nRF52840 PS v1.1 §§6.25.1/6.25.4: STOP completes the current byte,
    /// STOPPED means stopped, END means both DMA buffers are no longer accessed.
    pub struct DisplaySpi<'d>(CancelSafe<spim::Spim<'d>, Spi2Control>);

    impl<'d> DisplaySpi<'d> {
        pub fn new(
            spi: Peri<'d, peripherals::SPI2>,
            irq: impl interrupt::typelevel::Binding<
                    interrupt::typelevel::SPI2,
                    spim::InterruptHandler<peripherals::SPI2>,
                > + 'd,
            sck: Peri<'d, peripherals::P0_20>,
            mosi: Peri<'d, peripherals::P0_17>,
            config: spim::Config,
        ) -> Self {
            let spi = spim::Spim::new_txonly(spi, irq, sck, mosi, config);
            // SAFETY: construction consumes the unique SPI2 token, the abort
            // register block is exactly SPIM2, and HAL's END-only ISR does not
            // start transfers. The private wrapper prevents bypassing guards.
            Self(unsafe { CancelSafe::new(spi, Spi2Control) })
        }
    }

    impl embedded_hal::spi::ErrorType for DisplaySpi<'_> {
        type Error = spim::Error;
    }
    impl SpiBus<u8> for DisplaySpi<'_> {
        async fn read(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
            self.0.read(bytes).await
        }
        async fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
            self.0.write(bytes).await
        }
        async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
            self.0.transfer(read, write).await
        }
        async fn transfer_in_place(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
            self.0.transfer_in_place(bytes).await
        }
        async fn flush(&mut self) -> Result<(), Self::Error> {
            self.0.flush().await
        }
    }
}

#[cfg(target_os = "none")]
pub use nrf::DisplaySpi;
