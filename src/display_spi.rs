//! Cancellation boundary for embassy-nrf 0.11's otherwise unguarded SPIM DMA.

use core::{
    future::Future,
    pin::pin,
    sync::atomic::{Ordering, compiler_fence},
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
    // Drop the HAL driver before the control parks the released pins.
    spi: SPI,
    control: CONTROL,
}

/// Each opened bus must quiesce DMA on completion and cancellation before drop.
#[cfg(any(feature = "prototype-spim", test))]
pub(crate) trait SpiFactory {
    type Error: embedded_hal::spi::Error;
    type Bus<'a>: SpiBus<u8, Error = Self::Error>
    where
        Self: 'a;
    fn open(&mut self) -> Self::Bus<'_>;
}

/// Keep only construction resources between operations, not an enabled driver.
#[cfg(any(feature = "prototype-spim", test))]
pub(crate) struct Transient<F>(F);

#[cfg(any(feature = "prototype-spim", test))]
impl<F> Transient<F> {
    pub(crate) fn new(factory: F) -> Self {
        Self(factory)
    }
}

#[cfg(any(feature = "prototype-spim", test))]
impl<F: SpiFactory> embedded_hal::spi::ErrorType for Transient<F> {
    type Error = F::Error;
}

#[cfg(any(feature = "prototype-spim", test))]
impl<F: SpiFactory> SpiBus<u8> for Transient<F> {
    async fn read(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let mut bus = self.0.open();
        bus.read(bytes).await?;
        bus.flush().await
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut bus = self.0.open();
        bus.write(bytes).await?;
        bus.flush().await
    }
    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        let mut bus = self.0.open();
        bus.transfer(read, write).await?;
        bus.flush().await
    }
    async fn transfer_in_place(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        let mut bus = self.0.open();
        bus.transfer_in_place(bytes).await?;
        bus.flush().await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        // Every operation flushes before releasing its driver.
        Ok(())
    }
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
        Peri,
        interrupt::{self, typelevel::Interrupt},
        peripherals, spim,
    };

    struct Spi2Control;

    #[cfg(feature = "prototype-spim")]
    impl Spi2Control {
        fn park_pins() {
            use embassy_nrf::pac::gpio::vals::{Dir, Input};
            let port = embassy_nrf::pac::P0;
            port.outclr().write(|w| {
                w.set_pin(17, true);
                w.set_pin(20, true);
            });
            for pin in [17, 20] {
                port.pin_cnf(pin).write(|w| {
                    w.set_dir(Dir::Output);
                    w.set_input(Input::Disconnect);
                });
            }
        }
    }

    #[cfg(feature = "prototype-spim")]
    impl Drop for Spi2Control {
        fn drop(&mut self) {
            // CancelSafe drops SPIM first: no peripheral still drives these pins.
            Self::park_pins();
        }
    }

    #[cfg(feature = "prototype-spim")]
    struct Factory<'d> {
        spi: Peri<'d, peripherals::SPI2>,
        sck: Peri<'d, peripherals::P0_20>,
        mosi: Peri<'d, peripherals::P0_17>,
        irq: crate::DisplayIrqs,
        config: spim::Config,
    }

    #[cfg(feature = "prototype-spim")]
    impl SpiFactory for Factory<'_> {
        type Error = spim::Error;
        type Bus<'a>
            = CancelSafe<spim::Spim<'a, embassy_nrf::mode::Async>, Spi2Control>
        where
            Self: 'a;

        fn open(&mut self) -> Self::Bus<'_> {
            let spi = spim::Spim::new_txonly(
                self.spi.reborrow(),
                self.sck.reborrow(),
                self.mosi.reborrow(),
                self.irq,
                self.config.clone(),
            );
            // SAFETY: reborrowing retains exclusive SPI2/pin ownership in this
            // factory; the control quiesces exactly this driver's DMA.
            unsafe { CancelSafe::new(spi, Spi2Control) }
        }
    }

    #[cfg(feature = "prototype-spim")]
    type Driver<'d> = Transient<Factory<'d>>;
    #[cfg(not(feature = "prototype-spim"))]
    type Driver<'d> = CancelSafe<spim::Spim<'d, embassy_nrf::mode::Async>, Spi2Control>;

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

    /// Exclusively owns SPI2, retaining the HAL driver only per operation when
    /// `prototype-spim` is enabled. Avoids the nRF52840
    /// SPIM3-specific anomaly 198 without reserving a RAM bank. No raw peripheral or
    /// inner HAL handle escapes. Normal transfers remain interrupt-driven;
    /// only cancellation synchronously waits for STOPPED (at most the current
    /// byte in normal hardware operation), without globally masking interrupts.
    ///
    /// Nordic nRF52840 PS v1.1 §§6.25.1/6.25.4: STOP completes the current byte,
    /// STOPPED means stopped, END means both DMA buffers are no longer accessed.
    pub struct DisplaySpi<'d>(Driver<'d>);

    impl<'d> DisplaySpi<'d> {
        pub fn new(
            spi: Peri<'d, peripherals::SPI2>,
            irq: crate::DisplayIrqs,
            sck: Peri<'d, peripherals::P0_20>,
            mosi: Peri<'d, peripherals::P0_17>,
            config: spim::Config,
        ) -> Self {
            #[cfg(feature = "prototype-spim")]
            {
                Spi2Control::park_pins();
                Self(Transient::new(Factory {
                    spi,
                    sck,
                    mosi,
                    irq,
                    config,
                }))
            }
            #[cfg(not(feature = "prototype-spim"))]
            {
                let spi = spim::Spim::new_txonly(spi, sck, mosi, irq, config);
                // SAFETY: construction consumes the unique SPI2 token, the abort
                // register block is exactly SPIM2, and HAL's END-only ISR does not
                // start transfers. The private wrapper prevents bypassing guards.
                Self(unsafe { CancelSafe::new(spi, Spi2Control) })
            }
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
