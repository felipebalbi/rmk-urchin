//! Role-aware status rendering and panel refresh policy, independent of RMK.

use core::{convert::Infallible, fmt::Write};
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_5X8},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, StrokeAlignment},
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

struct ScaledTarget<'a, D> {
    display: &'a mut D,
    origin: Point,
    scale: i32,
}

impl<D> OriginDimensions for ScaledTarget<'_, D> {
    fn size(&self) -> Size {
        Size::new(68 / self.scale as u32, 160 / self.scale as u32)
    }
}

impl<D: DrawTarget<Color = BinaryColor>> DrawTarget for ScaledTarget<'_, D> {
    type Color = BinaryColor;
    type Error = D::Error;

    fn draw_iter<I: IntoIterator<Item = Pixel<BinaryColor>>>(
        &mut self,
        pixels: I,
    ) -> Result<(), Self::Error> {
        for Pixel(point, color) in pixels {
            Rectangle::new(
                self.origin + point * self.scale,
                Size::new(self.scale as u32, self.scale as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(self.display)?;
        }
        Ok(())
    }
}

fn text<D: DrawTarget<Color = BinaryColor>>(
    display: &mut D,
    value: &str,
    origin: Point,
    scale: i32,
) -> Result<(), D::Error> {
    Text::with_baseline(
        value,
        Point::zero(),
        MonoTextStyle::new(&FONT_5X8, BinaryColor::On),
        Baseline::Top,
    )
    .draw(&mut ScaledTarget {
        display,
        origin,
        scale,
    })
    .map(|_| ())
}

fn draw_line<D: DrawTarget<Color = BinaryColor>>(
    display: &mut D,
    mut point: Point,
    end: Point,
) -> Result<(), D::Error> {
    let dx = (end.x - point.x).abs();
    let dy = -(end.y - point.y).abs();
    let step = Point::new(
        if point.x < end.x { 1 } else { -1 },
        if point.y < end.y { 1 } else { -1 },
    );
    let mut error = dx + dy;
    loop {
        Pixel(point, BinaryColor::On).draw(display)?;
        if point == end {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            point.x += step.x;
        }
        if twice <= dx {
            error += dx;
            point.y += step.y;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Field {
    Battery,
    Usb,
    Layer,
    Profile,
    Host,
    Split,
}

impl Field {
    fn bounds(self) -> Rectangle {
        let (x, y, width, height) = match self {
            Self::Battery => (6, 23, 46, 10),
            Self::Usb => (57, 23, 5, 10),
            Self::Layer => (6, 69, 56, 18),
            Self::Profile => (6, 116, 22, 18),
            Self::Host => (36, 110, 28, 28),
            Self::Split => (6, 69, 56, 28),
        };
        Rectangle::new(Point::new(x, y), Size::new(width, height))
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
    sleeping: bool,
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
            sleeping: false,
        }
    }

    pub fn set_battery(&mut self, percentage: Option<u8>) -> bool {
        let value = percentage.map(|p| p.min(100));
        let changed = self.battery != value;
        self.battery = value;
        changed
    }

    pub fn set_sleeping(&mut self, sleeping: bool) -> bool {
        let changed = self.sleeping != sleeping;
        self.sleeping = sleeping;
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
                let _ = text.push_str("Nav");
            }
            2 => {
                let _ = text.push_str("Mouse");
            }
            3 => {
                let _ = text.push_str("Media");
            }
            4 => {
                let _ = text.push_str("Num");
            }
            5 => {
                let _ = text.push_str("Sym");
            }
            6 => {
                let _ = text.push_str("Fun");
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
        _now_us: u64,
    ) -> Result<(), D::Error> {
        display.clear(BinaryColor::Off)?;
        text(
            display,
            if self.role == Role::Central {
                "LEFT"
            } else {
                "RIGHT"
            },
            Point::new(6, 7),
            1,
        )?;
        Rectangle::new(Point::new(6, 41), Size::new(56, 1))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(display)?;
        if self.role == Role::Central {
            text(display, "LAYER", Point::new(6, 54), 1)?;
            text(display, "PROFILE", Point::new(6, 103), 1)?;
            text(display, "CENTRAL", Point::new(6, 146), 1)?;
        } else {
            text(display, "SPLIT LINK", Point::new(6, 54), 1)?;
            text(display, "TO LEFT", Point::new(16, 112), 1)?;
            text(display, "PERIPHERAL", Point::new(6, 146), 1)?;
        }
        self.render_fields(display, None)?;
        Ok(())
    }

    pub fn render_changes<D: DrawTarget<Color = BinaryColor>>(
        &self,
        display: &mut D,
        previous: &Self,
    ) -> Result<u128, D::Error> {
        self.render_fields(display, Some(previous))
    }

    fn render_fields<D: DrawTarget<Color = BinaryColor>>(
        &self,
        display: &mut D,
        previous: Option<&Self>,
    ) -> Result<u128, D::Error> {
        let central = self.role == Role::Central;
        let fields = [
            (
                Field::Battery,
                previous.is_none_or(|old| old.battery != self.battery),
            ),
            (
                Field::Usb,
                previous.is_none_or(|old| old.usb_powered != self.usb_powered),
            ),
            (
                Field::Layer,
                central && previous.is_none_or(|old| old.layer != self.layer),
            ),
            (
                Field::Profile,
                central && previous.is_none_or(|old| old.profile != self.profile),
            ),
            (
                Field::Host,
                central && previous.is_none_or(|old| old.host != self.host),
            ),
            (
                Field::Split,
                !central
                    && previous.is_none_or(|old| old.central_connected != self.central_connected),
            ),
        ];
        let mut rows = 0;
        for (field, changed) in fields {
            if !changed {
                continue;
            }
            let bounds = field.bounds();
            bounds
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
                .draw(display)?;
            self.draw_field(field, &mut display.clipped(&bounds))?;
            // Rotation maps portrait X directly to the physical gate-line address.
            rows |= ((1u128 << bounds.size.width) - 1) << bounds.top_left.x;
        }
        Ok(rows)
    }

    fn draw_field<D: DrawTarget<Color = BinaryColor>>(
        &self,
        field: Field,
        display: &mut D,
    ) -> Result<(), D::Error> {
        let white = PrimitiveStyle::with_fill(BinaryColor::On);
        match field {
            Field::Battery => {
                Rectangle::new(Point::new(6, 24), Size::new(15, 8))
                    .into_styled(
                        PrimitiveStyleBuilder::new()
                            .stroke_color(BinaryColor::On)
                            .stroke_width(1)
                            .stroke_alignment(StrokeAlignment::Inside)
                            .build(),
                    )
                    .draw(display)?;
                Rectangle::new(Point::new(21, 26), Size::new(2, 4))
                    .into_styled(white)
                    .draw(display)?;
                let mut value: String<4> = String::new();
                if let Some(level) = self.battery {
                    let width = 11 * u32::from(level) / 100;
                    if width != 0 {
                        Rectangle::new(Point::new(8, 26), Size::new(width, 4))
                            .into_styled(white)
                            .draw(display)?;
                    }
                    let _ = write!(value, "{level}%");
                } else {
                    text(display, "?", Point::new(11, 24), 1)?;
                    let _ = value.push_str("--%");
                }
                text(display, &value, Point::new(31, 24), 1)?;
            }
            Field::Usb if self.usb_powered => {
                for (a, b) in [
                    ((61, 23), (58, 27)),
                    ((58, 27), (61, 27)),
                    ((61, 27), (58, 31)),
                ] {
                    draw_line(display, Point::from(a), Point::from(b))?;
                }
            }
            Field::Usb => {}
            Field::Layer => {
                let mut value = self.layer_text();
                value.as_mut_str().make_ascii_uppercase();
                let scale = if value.len() <= 5 { 2 } else { 1 };
                let x = (68 - value.len() as i32 * 5 * scale) / 2;
                text(display, &value, Point::new(x, 70), scale)?;
            }
            Field::Profile => {
                let mut value: String<4> = String::new();
                let _ = write!(value, "P{}", self.profile);
                text(
                    display,
                    &value,
                    Point::new(6, 117),
                    if value.len() <= 2 { 2 } else { 1 },
                )?;
            }
            Field::Split => {
                let (value, scale) = if self.central_connected {
                    ("OK", 3)
                } else {
                    ("WAIT", 2)
                };
                text(
                    display,
                    value,
                    Point::new((68 - value.len() as i32 * 5 * scale) / 2, 70),
                    scale,
                )?;
            }
            Field::Host => {
                let origin = Point::new(45, 117);
                for (a, b) in [
                    ((4, 0), (4, 12)),
                    ((4, 0), (8, 4)),
                    ((8, 4), (0, 10)),
                    ((0, 2), (8, 8)),
                    ((8, 8), (4, 12)),
                ] {
                    draw_line(display, origin + Point::from(a), origin + Point::from(b))?;
                }
                if self.host == HostLink::Advertising {
                    for x in [43, 54] {
                        Rectangle::new(Point::new(x, 122), Size::new(2, 2))
                            .into_styled(white)
                            .draw(display)?;
                    }
                    for arc in [
                        [(5, 10), (4, 11), (3, 12), (3, 14), (4, 15), (5, 16)],
                        [(3, 8), (1, 10), (0, 12), (0, 14), (1, 16), (3, 18)],
                    ] {
                        for pair in arc.windows(2) {
                            for mirror in [false, true] {
                                let points = [pair[0], pair[1]].map(|(x, y)| {
                                    Point::new(36 + if mirror { 26 - x } else { x }, 110 + y)
                                });
                                draw_line(display, points[0], points[1])?;
                            }
                        }
                    }
                } else if self.host == HostLink::Inactive {
                    Rectangle::new(Point::new(51, 125), Size::new(11, 11))
                        .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
                        .draw(display)?;
                    for (a, b) in [((52, 126), (60, 134)), ((60, 126), (52, 134))] {
                        draw_line(display, Point::from(a), Point::from(b))?;
                    }
                }
            }
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
    powered: bool,
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
            powered: true,
        }
    }

    pub fn state_mut(&mut self) -> &mut ScreenState {
        &mut self.state
    }

    pub fn is_sleeping(&self) -> bool {
        self.state.sleeping
    }

    /// Apply the requested rail state only at a completed-transaction boundary.
    pub async fn sync_power(&mut self) -> Result<bool, SPI::Error> {
        if self.powered != self.state.sleeping {
            return Ok(false);
        }
        let result = if self.state.sleeping {
            let polarity = self.next_polarity;
            self.next_polarity = !polarity;
            self.panel.power_off(polarity).await
        } else {
            self.panel.power_on();
            Ok(())
        };
        self.powered = !self.state.sleeping;
        self.initialized = false;
        self.rendered = None;
        self.force_redraw = true;
        self.next_polarity = false;
        self.last_image_us = None;
        self.last_success = None;
        result.map(|()| true)
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
        self.sync_power().await?;
        if self.state.sleeping {
            return Ok(());
        }
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
        let force_redraw = self.force_redraw;
        let dirty = force_redraw || self.rendered != Some(self.state);
        let image_due = force_redraw
            || self
                .last_image_us
                .is_none_or(|last| now_us.saturating_sub(last) >= IMAGE_PERIOD_US);
        // Set before await so cancellation of either transaction also requests
        // a full resynchronization, not a maintenance-only recovery.
        self.force_redraw = true;
        if dirty && image_due {
            let snapshot = self.state;
            let rows = match self.rendered.as_ref() {
                Some(previous) if !force_redraw => snapshot
                    .render_changes(self.panel.framebuffer(), previous)
                    .unwrap(),
                _ => {
                    let _ = snapshot.render(self.panel.framebuffer(), now_us);
                    crate::nice_view::ALL_ROWS
                }
            };
            self.last_image_us = Some(now_us);
            if rows == crate::nice_view::ALL_ROWS {
                self.panel.flush(polarity).await?;
            } else {
                self.panel.flush_rows(polarity, rows).await?;
            }
            self.rendered = Some(snapshot);
            self.force_redraw = false;
        } else {
            self.panel.maintain(polarity).await?;
            self.force_redraw = force_redraw;
        }
        self.last_success = Some((now_us, polarity));
        Ok(())
    }
}
