//! Role-aware status rendering and panel refresh policy, independent of RMK.

use core::{convert::Infallible, fmt::Write};
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, ascii::FONT_5X8, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, spi::SpiBus};
use heapless::String;

use crate::nice_view::NiceView;

#[cfg(target_os = "none")]
pub mod runtime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Central,
    Peripheral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostLink {
    Inactive,
    Advertising,
    Connected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenState {
    role: Role,
    battery: Option<u8>,
    profile: u8,
    host: HostLink,
    central_connected: bool,
    layer: u8,
}

impl ScreenState {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            battery: None,
            profile: 0,
            host: HostLink::Inactive,
            central_connected: false,
            layer: 0,
        }
    }

    pub fn set_battery(&mut self, percentage: Option<u8>) -> bool {
        let value = percentage.map(|p| p.min(100));
        let changed = self.battery != value;
        self.battery = value;
        changed
    }

    pub fn set_host(&mut self, profile: u8, link: HostLink) -> bool {
        if self.role != Role::Central {
            return false;
        }
        let changed = self.profile != profile || self.host != link;
        self.profile = profile;
        self.host = link;
        changed
    }

    pub fn set_layer(&mut self, layer: u8) -> bool {
        if self.role != Role::Central {
            return false;
        }
        let changed = self.layer != layer;
        self.layer = layer;
        changed
    }

    pub fn set_central_connected(&mut self, connected: bool) -> bool {
        if self.role != Role::Peripheral {
            return false;
        }
        let changed = self.central_connected != connected;
        self.central_connected = connected;
        changed
    }

    pub fn battery_text(&self) -> String<4> {
        let mut text = String::new();
        match self.battery {
            Some(level) => {
                let _ = write!(text, "{}%", level);
            }
            None => {
                let _ = text.push_str("--%");
            }
        }
        text
    }

    pub fn layer_text(&self) -> String<12> {
        let mut text = String::new();
        match self.layer {
            0 => {
                let _ = text.push_str("Base");
            }
            1 => {
                let _ = text.push_str("Left");
            }
            2 => {
                let _ = text.push_str("Right");
            }
            3 => {
                let _ = text.push_str("Tri");
            }
            layer => {
                let _ = write!(text, "Layer {}", layer);
            }
        }
        text
    }

    pub fn render<D: DrawTarget<Color = BinaryColor>>(
        &self,
        display: &mut D,
    ) -> Result<(), D::Error> {
        display.clear(BinaryColor::On)?;
        let small = MonoTextStyle::new(&FONT_5X8, BinaryColor::Off);
        let large = MonoTextStyle::new(&FONT_10X20, BinaryColor::Off);
        Text::with_baseline("BATTERY", Point::new(3, 0), small, Baseline::Top).draw(display)?;
        Text::with_baseline(
            &self.battery_text(),
            Point::new(3, 12),
            large,
            Baseline::Top,
        )
        .draw(display)?;
        let label = match self.role {
            Role::Central => "BLUETOOTH",
            Role::Peripheral => "CENTRAL",
        };
        Text::with_baseline(label, Point::new(3, 42), small, Baseline::Top).draw(display)?;
        let link = match self.role {
            Role::Central => {
                let mut profile: String<12> = String::new();
                let _ = write!(profile, "Profile {}", self.profile);
                Text::with_baseline(&profile, Point::new(3, 54), small, Baseline::Top)
                    .draw(display)?;
                match self.host {
                    HostLink::Inactive => "Inactive",
                    HostLink::Advertising => "Advertising",
                    HostLink::Connected => "Connected",
                }
            }
            Role::Peripheral => {
                if self.central_connected {
                    "Connected"
                } else {
                    "Disconnected"
                }
            }
        };
        Text::with_baseline(link, Point::new(3, 66), small, Baseline::Top).draw(display)?;
        if self.role == Role::Central {
            Text::with_baseline("LAYER", Point::new(3, 104), small, Baseline::Top).draw(display)?;
            Text::with_baseline(&self.layer_text(), Point::new(3, 116), small, Baseline::Top)
                .draw(display)?;
        }
        Ok(())
    }
}

/// LD-29Y03 table 6-3-1 requires 57–70 frames/s with EXTMODE low.
/// Every transaction alternates polarity; unchanged content uses hold mode.
pub const PANEL_PERIOD_US: u64 = 16_000;
const IMAGE_PERIOD_US: u64 = 100_000;

/// Start-to-start deadlines, not a sleep after SPI completion. If a transaction
/// overruns, skip missed ticks instead of producing short catch-up COM phases.
pub struct PanelCadence {
    deadline_us: u64,
}

impl PanelCadence {
    pub fn new(now_us: u64) -> Self {
        Self {
            deadline_us: now_us,
        }
    }
    pub fn deadline_us(&self) -> u64 {
        self.deadline_us
    }
    pub fn completed(&mut self, started_us: u64, finished_us: u64) {
        let periods = (finished_us.saturating_sub(started_us) / PANEL_PERIOD_US) + 1;
        self.deadline_us = started_us + periods * PANEL_PERIOD_US;
    }
}

pub struct Screen<SPI, CS, POWER, DELAY> {
    panel: NiceView<SPI, CS, POWER, DELAY>,
    state: ScreenState,
    rendered: Option<ScreenState>,
    initialized: bool,
    force_redraw: bool,
    next_polarity: bool,
    last_image_us: Option<u64>,
    last_success: Option<(u64, bool)>,
}

impl<SPI, CS, POWER, DELAY> Screen<SPI, CS, POWER, DELAY>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
    POWER: OutputPin<Error = Infallible>,
    DELAY: DelayNs,
{
    pub fn new(panel: NiceView<SPI, CS, POWER, DELAY>, role: Role) -> Self {
        Self {
            panel,
            state: ScreenState::new(role),
            rendered: None,
            initialized: false,
            force_redraw: true,
            next_polarity: false,
            last_image_us: None,
            last_success: None,
        }
    }

    pub fn state_mut(&mut self) -> &mut ScreenState {
        &mut self.state
    }

    /// Last successfully completed transaction's start time and commanded
    /// polarity. SPI success is not an acknowledgement from this write-only LCD.
    pub fn last_success(&self) -> Option<(u64, bool)> {
        self.last_success
    }

    /// Perform rail settling before starting the fixed transaction cadence.
    pub async fn settle(&mut self) {
        self.panel.settle().await;
    }

    pub async fn tick(&mut self, now_us: u64) -> Result<(), SPI::Error> {
        let polarity = self.next_polarity;
        // Even failed initialization attempts may have delivered the command.
        self.next_polarity = !polarity;
        if !self.initialized {
            self.panel.initialize(polarity).await?;
            self.initialized = true;
            self.last_success = Some((now_us, polarity));
            return Ok(());
        }
        // There is no readback: on failure the polarity on glass is unknown.
        // Keep alternating attempts and force a complete image after recovery.
        let dirty = self.force_redraw || self.rendered != Some(self.state);
        let image_due = self
            .last_image_us
            .is_none_or(|last| now_us.saturating_sub(last) >= IMAGE_PERIOD_US);
        // Set before await so cancellation of either transaction also requests
        // a full resynchronization, not a maintenance-only recovery.
        self.force_redraw = true;
        if dirty && image_due {
            let snapshot = self.state;
            let _ = snapshot.render(self.panel.framebuffer());
            self.last_image_us = Some(now_us);
            self.panel.flush(polarity).await?;
            self.rendered = Some(snapshot);
        } else {
            self.panel.maintain(polarity).await?;
        }
        self.force_redraw = dirty && !image_due;
        self.last_success = Some((now_us, polarity));
        Ok(())
    }
}
