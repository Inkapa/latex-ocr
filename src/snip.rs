//! Full-screen transparent selection overlay for the snipping tool.
//!
//! The overlay is a borderless, always-on-top, transparent viewport that sits
//! on top of the live desktop. It dims everything except the drag rectangle
//! and reports the selected region back to the app when the mouse is released
//! or Escape is pressed.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Color32, CursorIcon, Key, Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId, pos2, vec2,
};

use crate::snapshot::{self, MonitorInfo, Region};
use crate::theme::ACCENT;

/// How the user finished the selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayOutcome {
    /// A valid region was selected, in global physical pixels.
    Captured(Region),
    /// The user pressed Escape or cancelled.
    Cancelled,
}

/// Shared state between the app and the overlay viewport.
pub struct OverlayState {
    /// Fresh id for this snip's viewport, so a new overlay never reuses stale
    /// window state from a previous snip.
    pub viewport_id: ViewportId,
    /// The monitor the overlay currently covers.
    monitor: MonitorInfo,
    /// Set once the user finishes the selection.
    pub outcome: Option<OverlayOutcome>,
    /// Button state from the previous frame, used to detect press/release edges.
    prev_down: bool,
    /// Whether the user is currently dragging a selection.
    dragging: bool,
    /// Selection corners in global physical pixels.
    start: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    /// The overlay has rendered at least one frame in its own window.
    pub rendered_once: bool,
    /// Consecutive frames the window geometry did not match its monitor.
    /// Used to fall back to drawing even if the correction never settles.
    unsettled: u32,
    /// Total frames rendered, for throttling diagnostics.
    tick_count: u64,
}

/// How many points of deviation is tolerated before the overlay is treated as
/// misplaced. Also the threshold past which a stale geometry is drawn anyway.
const GEOMETRY_TOLERANCE: f32 = 2.0;

/// If the overlay is stuck not matching its monitor for this many frames,
/// draw anyway so the snip does not silently become unusable.
const UNSETTLED_DRAW_FALLBACK: u32 = 90;

/// The smallest selection, in physical pixels, that counts as a capture.
const MIN_SELECTION: u32 = 8;

impl OverlayState {
    /// Starts a snip on the given monitor and returns the shared state.
    pub fn begin(monitor: MonitorInfo) -> Arc<Mutex<Self>> {
        log::debug!(
            "snip begin: monitor x={} y={} {}x{} scale={}",
            monitor.x,
            monitor.y,
            monitor.width,
            monitor.height,
            monitor.scale
        );
        Arc::new(Mutex::new(Self {
            viewport_id: fresh_viewport_id(),
            monitor,
            outcome: None,
            // The "Snip & OCR" button is still held when we get here, so the
            // first press must not start a selection.
            prev_down: snapshot::left_button_down(),
            dragging: false,
            start: None,
            current: None,
            rendered_once: false,
            unsettled: 0,
            tick_count: 0,
        }))
    }

    /// The monitor this overlay currently covers.
    pub fn monitor(&self) -> MonitorInfo {
        self.monitor
    }
}

fn fresh_viewport_id() -> ViewportId {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    ViewportId::from_hash_of(("latex-ocr-snip", n))
}

fn viewport_builder(monitor: MonitorInfo) -> ViewportBuilder {
    let position = pos2(
        monitor.x as f32 / monitor.scale,
        monitor.y as f32 / monitor.scale,
    );
    let size = vec2(
        monitor.width as f32 / monitor.scale,
        monitor.height as f32 / monitor.scale,
    );
    ViewportBuilder::default()
        .with_title("LaTeX OCR snip")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_position(position)
        .with_inner_size(size)
}

/// Shows the overlay viewport. Call once per frame while the snip is active.
pub fn show_viewport(ctx: &egui::Context, state: &Arc<Mutex<OverlayState>>) {
    let (viewport_id, builder) = {
        let state = state.lock().unwrap();
        (state.viewport_id, viewport_builder(state.monitor))
    };
    let state = Arc::clone(state);
    ctx.show_viewport_deferred(viewport_id, builder, move |ui, _class| {
        tick(ui, &state);
    });
}

fn tick(ui: &mut egui::Ui, state: &Arc<Mutex<OverlayState>>) {
    let ctx = ui.ctx().clone();

    let mut s = state.lock().unwrap();
    s.rendered_once = true;

    // The app is tearing us down after the selection finished.
    if s.outcome.is_some() {
        return;
    }

    // Closed by the OS (e.g. Alt+F4) without a selection.
    if ctx.input(|i| i.viewport().close_requested()) {
        s.outcome = Some(OverlayOutcome::Cancelled);
        ctx.send_viewport_cmd(ViewportCommand::Close);
        return;
    }

    let (global_cursor, down) = pointer_state(&ctx, s.monitor);

    // Follow the cursor onto other monitors before the user presses.
    if !s.dragging
        && let Some((cx, cy)) = global_cursor
        && !s.monitor.contains(cx, cy)
        && let Ok(monitor) = snapshot::monitor_at(cx, cy)
    {
        s.monitor = monitor;
    }
    ensure_monitor_geometry(&ctx, s.monitor);

    // Track the drag from the raw button state so it survives the cursor
    // leaving the window.
    let pressed = down && !s.prev_down;
    let released = !down && s.prev_down;
    s.prev_down = down;

    if pressed
        && !s.dragging
        && let Some((cx, cy)) = global_cursor
        && s.monitor.contains(cx, cy)
    {
        s.dragging = true;
        s.start = Some((cx, cy));
        s.current = Some((cx, cy));
    }
    if s.dragging {
        if let Some((cx, cy)) = global_cursor {
            s.current = Some((cx, cy));
        }
        if released {
            s.dragging = false;
            let start = s.start.unwrap_or((0, 0));
            let current = s.current.unwrap_or(start);
            s.outcome = Some(match selection_region(start, current, s.monitor) {
                Some(region) => OverlayOutcome::Captured(region),
                None => OverlayOutcome::Cancelled,
            });
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        s.outcome = Some(OverlayOutcome::Cancelled);
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    let settled = geometry_settled(&ctx, s.monitor);
    let force_draw = s.unsettled >= UNSETTLED_DRAW_FALLBACK;
    if settled {
        s.unsettled = 0;
    } else {
        s.unsettled = s.unsettled.saturating_add(1);
    }
    s.tick_count = s.tick_count.wrapping_add(1);
    if s.tick_count.is_multiple_of(60) {
        let viewports: Vec<_> = ctx.input(|i| i.raw.viewports.keys().copied().collect());
        if viewports.len() > 2 {
            log::debug!("snip: unexpected extra viewports alive: {viewports:?}");
        }
    }

    let (monitor, dragging, start, current) = (s.monitor, s.dragging, s.start, s.current);
    drop(s);

    draw(
        ui,
        &monitor,
        dragging,
        start,
        current,
        settled || force_draw,
    );

    // Keep polling the cursor and button even while the pointer is idle.
    ctx.request_repaint_after(Duration::from_millis(16));
}

/// The pointer position in global physical pixels plus the left button state.
/// On Windows the OS is polled directly; elsewhere egui input is used.
fn pointer_state(ctx: &egui::Context, monitor: MonitorInfo) -> (Option<(i32, i32)>, bool) {
    match snapshot::cursor_position() {
        Some(pos) => (Some(pos), snapshot::left_button_down()),
        None => {
            let down = ctx.input(|i| i.pointer.primary_down());
            let global = ctx
                .input(|i| i.pointer.latest_pos())
                .map(|local| to_global(ctx, monitor, local));
            (global, down)
        }
    }
}

/// The logical (points) size the overlay must have to exactly cover `monitor`.
fn target_logical_size(monitor: MonitorInfo) -> Vec2 {
    let scale = monitor.scale.max(0.1);
    vec2(monitor.width as f32 / scale, monitor.height as f32 / scale)
}

/// Keeps the overlay window covering exactly `monitor`'s physical bounds.
///
/// The position is corrected in physical space through the window's current
/// pixels-per-point, so the move is exact. The size is corrected in logical
/// space to the target monitor's scale: Windows rescales a window to keep its
/// logical size when it crosses a DPI boundary, so keeping the logical size at
/// `physical / scale` makes the window land at the correct physical size on the
/// new monitor. Correcting in logical space also catches crossings between
/// monitors that share a physical size but differ in scale, which a physical
/// comparison would miss and which would otherwise leave the window at a wrong
/// DPI-scaled size.
fn ensure_monitor_geometry(ctx: &egui::Context, monitor: MonitorInfo) {
    let ppp = ctx.input(|i| i.pixels_per_point).max(0.1);
    let Some(position_drift) = position_drift(ctx, monitor) else {
        return; // no geometry reported yet, e.g. before the first frame
    };
    if position_drift.x.abs() > GEOMETRY_TOLERANCE || position_drift.y.abs() > GEOMETRY_TOLERANCE {
        log::debug!(
            "snip geometry: pos {}x{} -> want {}x{} (ppp {ppp})",
            (monitor.x as f32 + position_drift.x).round(),
            (monitor.y as f32 + position_drift.y).round(),
            monitor.x,
            monitor.y
        );
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(
            monitor.x as f32 / ppp,
            monitor.y as f32 / ppp,
        )));
    }
    let Some(inner_rect) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    let target = target_logical_size(monitor);
    let logical_drift = vec2(inner_rect.width(), inner_rect.height()) - target;
    if logical_drift.x.abs() > GEOMETRY_TOLERANCE || logical_drift.y.abs() > GEOMETRY_TOLERANCE {
        log::debug!(
            "snip geometry: size {}x{} -> want {}x{} (scale {})",
            inner_rect.width().round(),
            inner_rect.height().round(),
            target.x.round(),
            target.y.round(),
            monitor.scale
        );
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(target));
    }
}

/// The window's actual physical position and size, as reported by the OS.
fn window_geometry(ctx: &egui::Context) -> Option<(Vec2, Vec2)> {
    let ppp = ctx.input(|i| i.pixels_per_point).max(0.1);
    ctx.input(|i| i.viewport().inner_rect).map(|inner| {
        let position = vec2(inner.min.x * ppp, inner.min.y * ppp);
        let size = vec2(inner.width() * ppp, inner.height() * ppp);
        (position, size)
    })
}

/// How far the window's physical position is from `monitor`, in pixels.
fn position_drift(ctx: &egui::Context, monitor: MonitorInfo) -> Option<Vec2> {
    let (position, _) = window_geometry(ctx)?;
    Some(position - vec2(monitor.x as f32, monitor.y as f32))
}

/// Whether the window currently covers `monitor` closely enough to be useful.
fn geometry_settled(ctx: &egui::Context, monitor: MonitorInfo) -> bool {
    let position_ok = position_drift(ctx, monitor).is_some_and(|drift| {
        drift.x.abs() <= GEOMETRY_TOLERANCE && drift.y.abs() <= GEOMETRY_TOLERANCE
    });
    let size_ok = ctx.input(|i| i.viewport().inner_rect).is_some_and(|inner| {
        let target = target_logical_size(monitor);
        (inner.width() - target.x).abs() <= GEOMETRY_TOLERANCE
            && (inner.height() - target.y).abs() <= GEOMETRY_TOLERANCE
    });
    position_ok && size_ok
}

/// Maps a point in the overlay's local points to global physical pixels.
fn to_global(ctx: &egui::Context, monitor: MonitorInfo, local: Pos2) -> (i32, i32) {
    let ppp = ctx.input(|i| i.pixels_per_point).max(0.1);
    let origin = ctx
        .input(|i| i.viewport().inner_rect)
        .map(|rect| rect.min)
        .unwrap_or_else(|| {
            pos2(
                monitor.x as f32 / monitor.scale,
                monitor.y as f32 / monitor.scale,
            )
        });
    (
        ((origin.x + local.x) * ppp).round() as i32,
        ((origin.y + local.y) * ppp).round() as i32,
    )
}

/// Maps global physical pixels to a point in the overlay's local points.
fn to_local(ctx: &egui::Context, monitor: MonitorInfo, x: i32, y: i32) -> Pos2 {
    let ppp = ctx.input(|i| i.pixels_per_point).max(0.1);
    let origin = ctx
        .input(|i| i.viewport().inner_rect)
        .map(|rect| rect.min)
        .unwrap_or_else(|| {
            pos2(
                monitor.x as f32 / monitor.scale,
                monitor.y as f32 / monitor.scale,
            )
        });
    pos2(x as f32 / ppp - origin.x, y as f32 / ppp - origin.y)
}

/// The captured region between two global physical points, clamped to the
/// monitor. `None` when the selection is too small to be worth capturing.
fn selection_region(
    start: (i32, i32),
    current: (i32, i32),
    monitor: MonitorInfo,
) -> Option<Region> {
    let max_x = monitor.x + monitor.width as i32;
    let max_y = monitor.y + monitor.height as i32;
    let left = start.0.min(current.0).clamp(monitor.x, max_x);
    let right = start.0.max(current.0).clamp(monitor.x, max_x);
    let top = start.1.min(current.1).clamp(monitor.y, max_y);
    let bottom = start.1.max(current.1).clamp(monitor.y, max_y);
    let width = (right - left) as u32;
    let height = (bottom - top) as u32;
    (width >= MIN_SELECTION && height >= MIN_SELECTION).then_some(Region {
        x: left,
        y: top,
        width,
        height,
    })
}

fn draw(
    ui: &mut egui::Ui,
    monitor: &MonitorInfo,
    dragging: bool,
    start: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    visible: bool,
) {
    let ctx = ui.ctx();
    ctx.set_cursor_icon(CursorIcon::Crosshair);

    // While the window is still moving onto the monitor under the cursor it is
    // left fully transparent, so a wrongly sized or placed frame never flashes
    // on screen.
    if !visible {
        return;
    }

    let screen = ui.max_rect();
    let dim = Color32::from_black_alpha(60);
    let painter = ui.painter();

    let selection = match (dragging, start, current) {
        (true, Some((sx, sy)), Some((cx, cy))) => {
            let a = to_local(ctx, *monitor, sx, sy);
            let b = to_local(ctx, *monitor, cx, cy);
            Some(Rect::from_two_pos(a, b).intersect(screen))
        }
        _ => None,
    };

    match selection {
        Some(rect) => {
            let top = Rect::from_min_max(screen.min, pos2(screen.max.x, rect.min.y));
            let bottom = Rect::from_min_max(pos2(screen.min.x, rect.max.y), screen.max);
            let left =
                Rect::from_min_max(pos2(screen.min.x, rect.min.y), pos2(rect.min.x, rect.max.y));
            let right =
                Rect::from_min_max(pos2(rect.max.x, rect.min.y), pos2(screen.max.x, rect.max.y));
            for strip in [top, bottom, left, right] {
                painter.rect_filled(strip, 0.0, dim);
            }
            // The border is drawn outside the selection so its anti-aliased
            // edge never leaks into the captured pixels.
            painter.rect_stroke(rect, 0.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
            draw_size_label(ui, rect);
        }
        None => {
            painter.rect_filled(screen, 0.0, dim);
        }
    }
}

/// Draws a small readout of the selection size just outside its corner.
fn draw_size_label(ui: &egui::Ui, selection: Rect) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let padding = vec2(4.0, 2.0);

    let size = painter_size(ui, selection);
    let text = format!("{} \u{00D7} {}", size.0, size.1);
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text, font.clone(), Color32::WHITE);
    let label_size = galley.size() + padding * 2.0;

    let screen = ui.max_rect();
    let mut pos = selection.max + vec2(6.0, 6.0);
    if pos.y + label_size.y > screen.max.y {
        pos.y = (selection.min.y - label_size.y - 6.0).max(screen.min.y);
    }
    if pos.x + label_size.x > screen.max.x {
        pos.x = (selection.max.x - label_size.x - 6.0).max(screen.min.x);
    }

    let label = Rect::from_min_size(pos, label_size);
    painter.rect_filled(label, 2.0, Color32::from_rgb(0x0F, 0x0F, 0x13));
    painter.galley(label.min + padding, galley, Color32::WHITE);
}

/// The selection size in physical pixels.
fn painter_size(ui: &egui::Ui, selection: Rect) -> (u32, u32) {
    let ppp = ui.ctx().input(|i| i.pixels_per_point).max(0.1);
    (
        (selection.width() * ppp).round().max(0.0) as u32,
        (selection.height() * ppp).round().max(0.0) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> MonitorInfo {
        MonitorInfo {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
        }
    }

    #[test]
    fn selects_a_normal_region() {
        let region = selection_region((100, 100), (500, 400), monitor()).unwrap();
        assert_eq!(region.x, 100);
        assert_eq!(region.y, 100);
        assert_eq!(region.width, 400);
        assert_eq!(region.height, 300);
    }

    #[test]
    fn normalizes_drag_direction() {
        let region = selection_region((500, 400), (100, 100), monitor()).unwrap();
        assert_eq!((region.x, region.y), (100, 100));
        assert_eq!((region.width, region.height), (400, 300));
    }

    #[test]
    fn clamps_to_the_monitor() {
        let region = selection_region((-50, -50), (2000, 1100), monitor()).unwrap();
        assert_eq!((region.x, region.y), (0, 0));
        assert_eq!((region.width, region.height), (1920, 1080));
    }

    #[test]
    fn rejects_a_tiny_click() {
        assert!(selection_region((100, 100), (103, 103), monitor()).is_none());
    }

    #[test]
    fn handles_monitors_with_negative_origin() {
        let left = MonitorInfo {
            x: -1280,
            y: 0,
            width: 1280,
            height: 720,
            scale: 1.0,
        };
        let region = selection_region((-1200, 100), (-100, 600), left).unwrap();
        assert_eq!(region.x, -1200);
        assert_eq!(region.width, 1100);
    }
}
