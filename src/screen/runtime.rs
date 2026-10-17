//! RMK adapters. Subscriptions are created in the macro's initialization block,
//! before ADC/BLE tasks start, and retained across every panel tick.

use core::{convert::Infallible, marker::PhantomData};
use embassy_futures::select::{select, select3, Either, Either3};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, spi::SpiBus};
use rmk::{
    core_traits::Runnable,
    event::{
        BatteryStatusEvent, CentralConnectedEvent, ConnectionStatusChangeEvent, EventSubscriber,
        LayerChangeEvent, SubscribableEvent,
    },
    processor::{PollingProcessor, Processor},
    types::{battery::BatteryStatus, ble::BleState},
};

use crate::{
    nice_view::NiceView,
    screen::{HostLink, PanelCadence, Role, Screen, ScreenState, PANEL_PERIOD_US},
};

pub enum ScreenEvent {
    Battery(BatteryStatusEvent),
    Host(ConnectionStatusChangeEvent),
    Layer(LayerChangeEvent),
    Central(CentralConnectedEvent),
}

impl ScreenEvent {
    fn apply(self, state: &mut ScreenState) {
        match self {
            Self::Battery(event) => {
                state.set_battery(match event.0 {
                    BatteryStatus::Unavailable => None,
                    BatteryStatus::Available { level, .. } => level,
                });
            }
            Self::Host(event) => {
                let link = match event.0.ble.state {
                    BleState::Inactive => HostLink::Inactive,
                    BleState::Advertising => HostLink::Advertising,
                    BleState::Connected => HostLink::Connected,
                };
                state.set_host(event.0.ble.profile, link);
            }
            Self::Layer(event) => {
                state.set_layer(event.0);
            }
            Self::Central(event) => {
                state.set_central_connected(event.connected);
            }
        }
    }
}

pub enum Subscriptions {
    Central {
        battery: <BatteryStatusEvent as SubscribableEvent>::Subscriber,
        host: <ConnectionStatusChangeEvent as SubscribableEvent>::Subscriber,
        layer: <LayerChangeEvent as SubscribableEvent>::Subscriber,
    },
    Peripheral {
        battery: <BatteryStatusEvent as SubscribableEvent>::Subscriber,
        central: <CentralConnectedEvent as SubscribableEvent>::Subscriber,
    },
}

impl Subscriptions {
    pub fn new(role: Role) -> Self {
        match role {
            Role::Central => Self::Central {
                battery: BatteryStatusEvent::subscriber(),
                host: ConnectionStatusChangeEvent::subscriber(),
                layer: LayerChangeEvent::subscriber(),
            },
            Role::Peripheral => Self::Peripheral {
                battery: BatteryStatusEvent::subscriber(),
                central: CentralConnectedEvent::subscriber(),
            },
        }
    }

    /// Drain bounded channels without awaiting: bursts collapse to the latest
    /// visible state before drawing, and cannot starve the next COM deadline.
    pub fn drain_into(&mut self, state: &mut ScreenState) {
        match self {
            Self::Central {
                battery,
                host,
                layer,
            } => {
                while let Some(event) = battery.try_next_message_pure() {
                    ScreenEvent::Battery(event).apply(state);
                }
                while let Some(event) = host.try_next_message_pure() {
                    ScreenEvent::Host(event).apply(state);
                }
                while let Some(event) = layer.try_next_message_pure() {
                    ScreenEvent::Layer(event).apply(state);
                }
            }
            Self::Peripheral { battery, central } => {
                while let Some(event) = battery.try_next_message_pure() {
                    ScreenEvent::Battery(event).apply(state);
                }
                while let Some(event) = central.try_next_message_pure() {
                    ScreenEvent::Central(event).apply(state);
                }
            }
        }
    }
}

impl EventSubscriber for Subscriptions {
    type Event = ScreenEvent;

    async fn next_event(&mut self) -> Self::Event {
        match self {
            Self::Central {
                battery,
                host,
                layer,
            } => match select3(battery.next_event(), host.next_event(), layer.next_event()).await {
                Either3::First(event) => ScreenEvent::Battery(event),
                Either3::Second(event) => ScreenEvent::Host(event),
                Either3::Third(event) => ScreenEvent::Layer(event),
            },
            Self::Peripheral { battery, central } => {
                match select(battery.next_event(), central.next_event()).await {
                    Either::First(event) => ScreenEvent::Battery(event),
                    Either::Second(event) => ScreenEvent::Central(event),
                }
            }
        }
    }
}

mod sealed {
    pub trait Sealed {}
}

pub trait ScreenRole: sealed::Sealed {
    const ROLE: Role;
}

// This shared module is compiled separately into each half's binary, where
// only that half's role marker is constructed.
#[allow(dead_code)]
pub struct Central;
#[allow(dead_code)]
pub struct Peripheral;

impl sealed::Sealed for Central {}
impl sealed::Sealed for Peripheral {}
impl ScreenRole for Central {
    const ROLE: Role = Role::Central;
}
impl ScreenRole for Peripheral {
    const ROLE: Role = Role::Peripheral;
}

pub struct ScreenProcessor<SPI, CS, POWER, DELAY, R: ScreenRole> {
    screen: Screen<SPI, CS, POWER, DELAY>,
    subscriptions: Subscriptions,
    usb_power_present: fn() -> bool,
    failed: bool,
    _role: PhantomData<R>,
}

impl<SPI, CS, POWER, DELAY> ScreenProcessor<SPI, CS, POWER, DELAY, Central>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    /// Call after storage/keymap initialization, before running the screen.
    /// Restoring behavior.default_layer does not publish a LayerChangeEvent.
    #[allow(dead_code)] // Central-only hook; shared source also builds into peripheral.
    pub fn seed_from_keymap(&mut self, keymap: &rmk::keymap::KeyMap<'_>) {
        self.screen.state_mut().set_layer(keymap.active_layer());
    }
}

impl<SPI, CS, POWER, DELAY, R: ScreenRole> ScreenProcessor<SPI, CS, POWER, DELAY, R>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    pub fn new(panel: NiceView<SPI, CS, POWER, DELAY>, usb_power_present: fn() -> bool) -> Self {
        Self {
            subscriptions: Subscriptions::new(R::ROLE),
            screen: Screen::new(panel, R::ROLE),
            usb_power_present,
            failed: false,
            _role: PhantomData,
        }
    }
}

impl<SPI, CS, POWER, DELAY, R: ScreenRole> Runnable for ScreenProcessor<SPI, CS, POWER, DELAY, R>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    async fn run(&mut self) -> ! {
        self.polling_loop().await
    }
}

impl<SPI, CS, POWER, DELAY, R: ScreenRole> Processor for ScreenProcessor<SPI, CS, POWER, DELAY, R>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    type Event = ScreenEvent;

    fn subscriber() -> impl EventSubscriber<Event = Self::Event> {
        Subscriptions::new(R::ROLE)
    }
    async fn process(&mut self, event: Self::Event) {
        event.apply(self.screen.state_mut());
    }
    async fn process_loop(&mut self) -> ! {
        self.polling_loop().await
    }
}

impl<SPI, CS, POWER, DELAY, R: ScreenRole> PollingProcessor
    for ScreenProcessor<SPI, CS, POWER, DELAY, R>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    fn interval(&self) -> Duration {
        Duration::from_micros(PANEL_PERIOD_US)
    }

    async fn update(&mut self) {
        self.screen
            .state_mut()
            .set_usb_powered((self.usb_power_present)());
        self.subscriptions.drain_into(self.screen.state_mut());
        let previous_success = self.screen.last_success();
        match self.screen.tick(Instant::now().as_micros()).await {
            Ok(()) => {
                if self.failed {
                    let gap_us = previous_success
                        .map(|(at, _)| Instant::now().as_micros().saturating_sub(at));
                    defmt::info!(
                        "nice!view SPI recovered; gap since prior success (us): {:?}",
                        gap_us
                    );
                }
                self.failed = false;
            }
            Err(_) => {
                if !self.failed {
                    defmt::warn!("nice!view SPI failed; retaining redraw and retrying");
                }
                self.failed = true;
            }
        }
    }

    async fn polling_loop(&mut self) -> ! {
        // Do not use RMK's default polling_loop: it constructs subscriptions
        // too late, and repeated event delivery could delay maintenance.
        self.screen.settle().await;
        let mut cadence = PanelCadence::new(Instant::now().as_micros());
        loop {
            Timer::at(Instant::from_micros(cadence.deadline_us())).await;
            let started = Instant::now().as_micros();
            self.update().await;
            cadence.completed(started, Instant::now().as_micros());
        }
    }
}
