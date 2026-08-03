//! Screen capture for the snipping tool.

use image::RgbaImage;
use xcap::Monitor;

/// A rectangular region of the virtual screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Physical geometry of a single monitor, in virtual-screen coordinates.
/// Monitors left or above the primary have negative `x` or `y`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl MonitorInfo {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x + self.width as i32
            && y < self.y + self.height as i32
    }
}

/// Lists all monitors with their geometry and scale factors.
pub fn monitors() -> Result<Vec<MonitorInfo>, String> {
    let mut out = Vec::new();
    for monitor in Monitor::all().map_err(|e| e.to_string())? {
        let info = MonitorInfo {
            x: monitor.x().map_err(|e| e.to_string())?,
            y: monitor.y().map_err(|e| e.to_string())?,
            width: monitor.width().map_err(|e| e.to_string())?,
            height: monitor.height().map_err(|e| e.to_string())?,
            scale: monitor.scale_factor().map_err(|e| e.to_string())?,
        };
        log::debug!(
            "monitor: x={} y={} {}x{} scale={}",
            info.x,
            info.y,
            info.width,
            info.height,
            info.scale
        );
        out.push(info);
    }
    if out.is_empty() {
        return Err("no monitors found".to_string());
    }
    Ok(out)
}

/// The monitor that contains the given point, or the first one if the point is
/// outside every monitor.
pub fn monitor_at(x: i32, y: i32) -> Result<MonitorInfo, String> {
    let all = monitors()?;
    Ok(all
        .iter()
        .copied()
        .find(|m| m.contains(x, y))
        .unwrap_or(all[0]))
}

/// The monitor the cursor is currently on, or the first one when the cursor
/// position is unknown.
pub fn monitor_at_cursor() -> Result<MonitorInfo, String> {
    let all = monitors()?;
    match cursor_position() {
        Some((x, y)) => Ok(all
            .iter()
            .copied()
            .find(|m| m.contains(x, y))
            .unwrap_or(all[0])),
        None => Ok(all[0]),
    }
}

/// A capture of the whole virtual desktop, with the origin that maps image
/// coordinates back to global physical pixels.
pub struct VirtualDesktop {
    pub image: RgbaImage,
    pub origin_x: i32,
    pub origin_y: i32,
}

/// Captures every monitor and stitches the result into one image covering the
/// whole virtual desktop. The union may have gaps where no monitor exists;
/// those stay transparent and are never displayed.
pub fn capture_virtual_desktop() -> Result<VirtualDesktop, String> {
    let all = monitors()?;
    let min_x = all.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = all.iter().map(|m| m.y).min().unwrap_or(0);
    let max_x = all.iter().map(|m| m.x + m.width as i32).max().unwrap_or(0);
    let max_y = all.iter().map(|m| m.y + m.height as i32).max().unwrap_or(0);
    let width = (max_x - min_x) as u32;
    let height = (max_y - min_y) as u32;
    if width == 0 || height == 0 {
        return Err("no screen area to capture".to_string());
    }
    // Capture the monitors concurrently, then stitch the results.
    let captures: Vec<(MonitorInfo, _)> = all
        .iter()
        .map(|m| {
            let m = *m;
            let handle = std::thread::spawn(move || {
                let monitor = Monitor::from_point(m.x, m.y).map_err(|e| e.to_string())?;
                monitor
                    .capture_region(0, 0, m.width, m.height)
                    .map_err(|e| format!("screen capture failed: {e}"))
            });
            (m, handle)
        })
        .collect();
    let mut out = RgbaImage::new(width, height);
    let mut first_error: Option<String> = None;
    for (m, handle) in captures {
        match handle.join() {
            Ok(Ok(img)) => {
                let ox = (m.x - min_x) as i64;
                let oy = (m.y - min_y) as i64;
                image::imageops::replace(&mut out, &img, ox, oy);
            }
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
            Err(_) => {
                if first_error.is_none() {
                    first_error = Some("screen capture thread panicked".to_string());
                }
            }
        }
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(VirtualDesktop {
        image: out,
        origin_x: min_x,
        origin_y: min_y,
    })
}

/// Captures a region of the live screen, in global physical pixels. The region
/// is clamped to the monitor that holds its top-left corner.
pub fn capture_region(region: Region) -> Result<RgbaImage, String> {
    if region.width == 0 || region.height == 0 {
        return Err("empty capture region".to_string());
    }
    let monitor = monitor_at(
        region.x + region.width as i32 / 2,
        region.y + region.height as i32 / 2,
    )?;

    let local_x = (region.x - monitor.x).clamp(0, monitor.width as i32) as u32;
    let local_y = (region.y - monitor.y).clamp(0, monitor.height as i32) as u32;
    let width = region.width.min(monitor.width - local_x);
    let height = region.height.min(monitor.height - local_y);
    if width == 0 || height == 0 {
        return Err("capture region is outside the monitor".to_string());
    }

    let monitor = Monitor::from_point(monitor.x, monitor.y).map_err(|e| e.to_string())?;
    monitor
        .capture_region(local_x, local_y, width, height)
        .map_err(|e| format!("screen capture failed: {e}"))
}

/// The cursor position in global physical pixels, when the platform can report it.
#[cfg(target_os = "windows")]
pub fn cursor_position() -> Option<(i32, i32)> {
    win::cursor_position()
}

/// The cursor position in global physical pixels, when the platform can report it.
#[cfg(not(target_os = "windows"))]
pub fn cursor_position() -> Option<(i32, i32)> {
    None
}

/// Whether the left mouse button is currently held down.
#[cfg(target_os = "windows")]
pub fn left_button_down() -> bool {
    win::left_button_down()
}

/// Whether the left mouse button is currently held down.
#[cfg(not(target_os = "windows"))]
pub fn left_button_down() -> bool {
    false
}

/// Minimal Windows FFI for polling the global cursor, kept dependency-free.
#[cfg(target_os = "windows")]
mod win {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    const VK_LBUTTON: i32 = 0x01;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    pub fn cursor_position() -> Option<(i32, i32)> {
        let mut point = Point::default();
        // SAFETY: `GetCursorPos` writes into the provided buffer.
        let ok = unsafe { GetCursorPos(&mut point) };
        (ok != 0).then_some((point.x, point.y))
    }

    pub fn left_button_down() -> bool {
        // SAFETY: `GetAsyncKeyState` is safe to call with any virtual-key code.
        (unsafe { GetAsyncKeyState(VK_LBUTTON) }) < 0
    }
}
