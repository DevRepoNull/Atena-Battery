//! Dynamic RGBA Battery Icon Generation for System Tray.
//!
//! Generates 32x32 RGBA pixel buffers in memory depicting a crisp battery casing
//! and color-coded internal fill corresponding to the battery level:
//! - Green (`#4CAF50`) for level > 50%
//! - Orange (`#FF9800`) for level 20%..=50%
//! - Red (`#F44336`) for level < 20%
//! - Slate Gray (`#9E9E9E`) for offline / unknown state

use tray_icon::Icon;

pub const ICON_WIDTH: u32 = 32;
pub const ICON_HEIGHT: u32 = 32;

/// RGBA Color tuple `(R, G, B, A)`.
type Rgba = (u8, u8, u8, u8);

const COLOR_BORDER: Rgba = (230, 230, 230, 255);
const COLOR_BORDER_DARK: Rgba = (140, 140, 140, 255);
const COLOR_BACKGROUND: Rgba = (0, 0, 0, 0);

const COLOR_GREEN: Rgba = (76, 175, 80, 255);
const COLOR_ORANGE: Rgba = (255, 152, 0, 255);
const COLOR_RED: Rgba = (244, 67, 54, 255);
const COLOR_GRAY: Rgba = (158, 158, 158, 255);

/// Returns the fill color for a given battery level and connection status.
pub fn get_fill_color(level: u8, is_connected: bool) -> Rgba {
    if !is_connected {
        COLOR_GRAY
    } else if level > 50 {
        COLOR_GREEN
    } else if level >= 20 {
        COLOR_ORANGE
    } else {
        COLOR_RED
    }
}

/// Generates a 32x32 RGBA byte buffer representing a battery icon.
pub fn generate_battery_rgba(level: u8, is_connected: bool) -> Vec<u8> {
    let mut pixels = vec![0u8; (ICON_WIDTH * ICON_HEIGHT * 4) as usize];

    let set_pixel = |buf: &mut [u8], x: u32, y: u32, color: Rgba| {
        if x < ICON_WIDTH && y < ICON_HEIGHT {
            let offset = ((y * ICON_WIDTH + x) * 4) as usize;
            buf[offset] = color.0;
            buf[offset + 1] = color.1;
            buf[offset + 2] = color.2;
            buf[offset + 3] = color.3;
        }
    };

    // 1. Clear background (transparent)
    for y in 0..ICON_HEIGHT {
        for x in 0..ICON_WIDTH {
            set_pixel(&mut pixels, x, y, COLOR_BACKGROUND);
        }
    }

    // Geometry parameters (Horizontal battery):
    // Main body: x: 3..25, y: 9..22
    // Terminal tip: x: 26..28, y: 13..18
    let body_x0 = 3u32;
    let body_x1 = 25u32;
    let body_y0 = 9u32;
    let body_y1 = 22u32;

    let tip_x0 = 26u32;
    let tip_x1 = 28u32;
    let tip_y0 = 13u32;
    let tip_y1 = 18u32;

    // 2. Draw outer border
    // Top & bottom lines of main body
    for x in body_x0..=body_x1 {
        set_pixel(&mut pixels, x, body_y0, COLOR_BORDER);
        set_pixel(&mut pixels, x, body_y1, COLOR_BORDER_DARK);
    }
    // Left & right lines of main body
    for y in body_y0..=body_y1 {
        set_pixel(&mut pixels, body_x0, y, COLOR_BORDER);
        set_pixel(&mut pixels, body_x1, y, COLOR_BORDER_DARK);
    }

    // Terminal tip (Positive node)
    for x in tip_x0..=tip_x1 {
        set_pixel(&mut pixels, x, tip_y0, COLOR_BORDER);
        set_pixel(&mut pixels, x, tip_y1, COLOR_BORDER_DARK);
    }
    for y in tip_y0..=tip_y1 {
        set_pixel(&mut pixels, tip_x1, y, COLOR_BORDER_DARK);
        // Fill inside tip
        for x in tip_x0..tip_x1 {
            set_pixel(&mut pixels, x, y, COLOR_BORDER);
        }
    }

    // 3. Draw internal battery fill
    let inner_x0 = body_x0 + 2;
    let inner_x1 = body_x1 - 2;
    let inner_y0 = body_y0 + 2;
    let inner_y1 = body_y1 - 2;

    let max_fill_width = inner_x1 - inner_x0 + 1; // 18 pixels
    let clamped_level = level.min(100) as u32;
    let fill_width = if is_connected {
        (clamped_level * max_fill_width).div_ceil(100) // Round up slightly for visibility
    } else {
        0
    };

    let fill_color = get_fill_color(level, is_connected);

    for y in inner_y0..=inner_y1 {
        for w in 0..max_fill_width {
            let x = inner_x0 + w;
            if w < fill_width {
                set_pixel(&mut pixels, x, y, fill_color);
            } else {
                // Dim unfilled interior
                set_pixel(&mut pixels, x, y, (40, 40, 40, 180));
            }
        }
    }

    pixels
}

/// Creates a `tray_icon::Icon` from a given battery level and status.
pub fn create_tray_icon(level: u8, is_connected: bool) -> Result<Icon, tray_icon::BadIcon> {
    let rgba = generate_battery_rgba(level, is_connected);
    Icon::from_rgba(rgba, ICON_WIDTH, ICON_HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_battery_rgba_buffer_size() {
        let rgba = generate_battery_rgba(75, true);
        assert_eq!(rgba.len(), (ICON_WIDTH * ICON_HEIGHT * 4) as usize);
    }

    #[test]
    fn test_fill_colors() {
        assert_eq!(get_fill_color(100, true), COLOR_GREEN);
        assert_eq!(get_fill_color(51, true), COLOR_GREEN);
        assert_eq!(get_fill_color(50, true), COLOR_ORANGE);
        assert_eq!(get_fill_color(20, true), COLOR_ORANGE);
        assert_eq!(get_fill_color(19, true), COLOR_RED);
        assert_eq!(get_fill_color(0, true), COLOR_RED);
        assert_eq!(get_fill_color(80, false), COLOR_GRAY);
    }

    #[test]
    fn test_create_tray_icon_success() {
        let icon_res = create_tray_icon(85, true);
        assert!(icon_res.is_ok());
    }
}
