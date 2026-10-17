//! Role-aware status rendering and panel refresh policy, independent of RMK.

use core::{convert::Infallible, fmt::Write};
use embedded_graphics::{
    mono_font::{ascii::FONT_5X8, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{
        Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, StrokeAlignment, Triangle,
    },
    text::{Baseline, Text},
};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::{delay::DelayNs, spi::SpiBus};
use heapless::String;

use crate::nice_view::NiceView;

#[cfg(target_os = "none")]
#[path = "screen/runtime.rs"]
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

/// Enlarge the existing bitmap font fourfold without a second font asset.
/// A profile uses at most three 5x8 characters, giving a 60x32 pixel number.
struct ProfileTarget<'a, D> {
    display: &'a mut D,
    origin: Point,
}

impl<D> OriginDimensions for ProfileTarget<'_, D> {
    fn size(&self) -> Size {
        Size::new(15, 8)
    }
}

impl<D: DrawTarget<Color = BinaryColor>> DrawTarget for ProfileTarget<'_, D> {
    type Color = BinaryColor;
    type Error = D::Error;

    fn draw_iter<I: IntoIterator<Item = Pixel<BinaryColor>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(point, color) in pixels {
            Rectangle::new(
                self.origin + Point::new(point.x * 4, point.y * 4),
                Size::new(4, 4),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(self.display)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenState {
    role: Role,
    battery: Option<u8>,
    usb_powered: bool,
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
            usb_powered: false,
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

    /// Local USB supply presence, not charger activity or charge completion.
    pub fn set_usb_powered(&mut self, powered: bool) -> bool {
        let changed = self.usb_powered != powered;
        self.usb_powered = powered;
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
        now_us: u64,
    ) -> Result<(), D::Error> {
        display.clear(BinaryColor::Off)?;
        let small = MonoTextStyle::new(&FONT_5X8, BinaryColor::On);
        Text::with_baseline("BATTERY", Point::new(3, 0), small, Baseline::Top).draw(display)?;
        if self.usb_powered {
            let white = PrimitiveStyle::with_fill(BinaryColor::On);
            Triangle::new(Point::new(57, 0), Point::new(50, 6), Point::new(55, 6))
                .into_styled(white)
                .draw(display)?;
            Triangle::new(Point::new(54, 4), Point::new(59, 4), Point::new(51, 11))
                .into_styled(white)
                .draw(display)?;
        }
        Rectangle::new(Point::new(3, 12), Size::new(56, 20))
            .into_styled(
                PrimitiveStyleBuilder::new()
                    .stroke_color(BinaryColor::On)
                    .stroke_width(2)
                    .stroke_alignment(StrokeAlignment::Inside)
                    .build(),
            )
            .draw(display)?;
        let white = PrimitiveStyle::with_fill(BinaryColor::On);
        Rectangle::new(Point::new(59, 18), Size::new(4, 8))
            .into_styled(white)
            .draw(display)?;
        match self.battery {
            Some(level) => {
                // Ten columns, each four pixels wide with a one-pixel gap.
                // Only complete 10% increments are filled; 100% fills all ten.
                for column in 0..level / 10 {
                    Rectangle::new(Point::new(6 + i32::from(column) * 5, 15), Size::new(4, 14))
                        .into_styled(white)
                        .draw(display)?;
                }
            }
            None => {
                Text::with_baseline("?", Point::new(29, 18), small, Baseline::Top).draw(display)?;
            }
        }
        let label = match self.role {
            Role::Central => "BLUETOOTH",
            Role::Peripheral => "CENTRAL",
        };
        Text::with_baseline(label, Point::new(3, 42), small, Baseline::Top).draw(display)?;
        if self.bluetooth_visible(now_us) {
            let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 2);
            // The standard Bluetooth rune, with solid strokes on the dark panel.
            for (start, end) in [
                ((56, 40), (56, 52)),
                ((56, 52), (62, 48)),
                ((62, 48), (52, 42)),
                ((52, 50), (62, 44)),
                ((62, 44), (56, 40)),
            ] {
                Line::new(Point::from(start), Point::from(end))
                    .into_styled(stroke)
                    .draw(display)?;
            }
            let disconnected = match self.role {
                Role::Central => self.host == HostLink::Inactive,
                Role::Peripheral => !self.central_connected,
            };
            if disconnected {
                // Clear behind the X so it remains legible over the rune.
                Rectangle::new(Point::new(59, 45), Size::new(9, 9))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
                    .draw(display)?;
                for (start, end) in [((60, 46), (66, 52)), ((60, 52), (66, 46))] {
                    Line::new(Point::from(start), Point::from(end))
                        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                        .draw(display)?;
                }
            }
        }
        if self.role == Role::Central {
            Text::with_baseline("LAYER", Point::new(3, 104), small, Baseline::Top).draw(display)?;
            Text::with_baseline(&self.layer_text(), Point::new(3, 116), small, Baseline::Top)
                .draw(display)?;
            let mut profile: String<3> = String::new();
            let _ = write!(profile, "{}", self.profile);
            let mut target = ProfileTarget {
                display,
                origin: Point::new(65 - profile.len() as i32 * 20, 124),
            };
            Text::with_baseline(&profile, Point::zero(), small, Baseline::Top).draw(&mut target)?;
        }
        Ok(())
    }

    fn bluetooth_visible(&self, now_us: u64) -> bool {
        self.role != Role::Central
            || self.host != HostLink::Advertising
            || (now_us / BLINK_PERIOD_US).is_multiple_of(2)
    }
}

/// LD-29Y03 table 6-3-1 requires 57–70 frames/s with EXTMODE low.
/// Every transaction alternates polarity; unchanged content uses hold mode.
pub const PANEL_PERIOD_US: u64 = 16_000;
const IMAGE_PERIOD_US: u64 = 100_000;
const BLINK_PERIOD_US: u64 = 500_000;

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
    rendered: Option<(ScreenState, bool)>,
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
        let visible = self.state.bluetooth_visible(now_us);
        let dirty = self.force_redraw || self.rendered != Some((self.state, visible));
        let image_due = self
            .last_image_us
            .is_none_or(|last| now_us.saturating_sub(last) >= IMAGE_PERIOD_US);
        // Set before await so cancellation of either transaction also requests
        // a full resynchronization, not a maintenance-only recovery.
        self.force_redraw = true;
        if dirty && image_due {
            let snapshot = self.state;
            let _ = snapshot.render(self.panel.framebuffer(), now_us);
            self.last_image_us = Some(now_us);
            self.panel.flush(polarity).await?;
            self.rendered = Some((snapshot, visible));
        } else {
            self.panel.maintain(polarity).await?;
        }
        self.force_redraw = dirty && !image_due;
        self.last_success = Some((now_us, polarity));
        Ok(())
    }
}
