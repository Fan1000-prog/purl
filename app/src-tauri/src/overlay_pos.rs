//! Where the overlay pill goes, as pure math over the numbers tao reports.
//!
//! macOS and Windows disagree on what a "global desktop coordinate" is, and
//! mixing the two up only shows on mixed-DPI setups (a Retina laptop beside a
//! 1x external, or a 125 % laptop beside a 100 % monitor), so it is kept here
//! where a test can pin it down.

/// The global coordinate space the OS lays its displays out in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopSpace {
    /// macOS: one global space in logical points. tao scales each monitor's
    /// rect by that monitor's own factor, and the cursor by the primary's.
    Points,
    /// Windows (per-monitor DPI aware): one global space in physical pixels.
    /// Cursor, monitor rects and window positions are already in it.
    Pixels,
}

/// This OS's desktop space.
pub const DESKTOP: DesktopSpace = if cfg!(target_os = "macos") {
    DesktopSpace::Points
} else {
    DesktopSpace::Pixels
};

/// Gap between the pill and the bottom of the work area, in logical points.
const BOTTOM_MARGIN: f64 = 8.0;

/// A rect as tao reports it: origin and size in that monitor's physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The cursor in desktop coordinates. `raw` is tao's `cursor_position()`,
/// which on macOS is the logical mouse location times the *primary* display's
/// scale, so the primary scale is divided back out there.
pub fn cursor_in_desktop(space: DesktopSpace, raw: (f64, f64), primary_scale: f64) -> (f64, f64) {
    match space {
        DesktopSpace::Points => (raw.0 / primary_scale, raw.1 / primary_scale),
        DesktopSpace::Pixels => raw,
    }
}

/// Top-left of the pill, in desktop coordinates: bottom-center of the
/// monitor's work area (the Dock and taskbar already excluded), `BOTTOM_MARGIN`
/// above its bottom edge. `pill` is the pill's size in logical points; it
/// renders at the target monitor's scale once it lands there.
pub fn pill_origin(
    space: DesktopSpace,
    work: RawRect,
    monitor_scale: f64,
    pill: (f64, f64),
) -> (f64, f64) {
    let (pw, ph) = (pill.0 * monitor_scale, pill.1 * monitor_scale);
    let margin = BOTTOM_MARGIN * monitor_scale;
    let x = work.x + (work.width - pw).max(0.0) / 2.0;
    let y = work.y + (work.height - ph - margin).max(0.0);
    match space {
        DesktopSpace::Points => (x / monitor_scale, y / monitor_scale),
        DesktopSpace::Pixels => (x.round(), y.round()),
    }
}

/// True when `cursor` (desktop coordinates) is within `margin` logical points
/// of `rect` ([x, y, width, height] in logical points, relative to the
/// window). `origin` is the window's `outer_position()` and `scale` its scale
/// factor.
pub fn cursor_over(
    space: DesktopSpace,
    cursor: (f64, f64),
    origin: (f64, f64),
    scale: f64,
    rect: [f64; 4],
    margin: f64,
) -> bool {
    // Bring the window's origin and the rect into the cursor's space.
    let (ox, oy, k) = match space {
        DesktopSpace::Points => (origin.0 / scale, origin.1 / scale, 1.0),
        DesktopSpace::Pixels => (origin.0, origin.1, scale),
    };
    let [x, y, w, h] = rect.map(|v| v * k);
    let m = margin * k;
    let (cx, cy) = cursor;
    cx >= ox + x - m && cx <= ox + x + w + m && cy >= oy + y - m && cy <= oy + y + h + m
}

#[cfg(test)]
mod tests {
    use super::*;

    const PILL: (f64, f64) = (200.0, 82.0);

    struct Monitor {
        scale: f64,
        work: RawRect,
    }

    /// Windows: a 125 % laptop (2400x1500 px, 60 px taskbar) with a 100 %
    /// monitor (1920x1080, 40 px taskbar) to its right. Windows lays both out
    /// in one physical-pixel space, so the second one starts at x = 2400.
    fn windows_desk() -> [Monitor; 2] {
        [
            Monitor {
                scale: 1.25,
                work: RawRect {
                    x: 0.0,
                    y: 0.0,
                    width: 2400.0,
                    height: 1440.0,
                },
            },
            Monitor {
                scale: 1.0,
                work: RawRect {
                    x: 2400.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1040.0,
                },
            },
        ]
    }

    /// macOS: a 2x Retina laptop (1440x900 pt, 25 pt menu bar) with a 1x
    /// external (1920x1080 pt, 25 pt menu bar) to its right at x = 1440 pt.
    /// tao reports each rect in points times that monitor's own scale.
    fn mac_desk() -> [Monitor; 2] {
        [
            Monitor {
                scale: 2.0,
                work: RawRect {
                    x: 0.0,
                    y: 50.0,
                    width: 2880.0,
                    height: 1750.0,
                },
            },
            Monitor {
                scale: 1.0,
                work: RawRect {
                    x: 1440.0,
                    y: 25.0,
                    width: 1920.0,
                    height: 1055.0,
                },
            },
        ]
    }

    #[test]
    fn windows_pill_lands_bottom_center_of_each_mixed_dpi_monitor() {
        let [laptop, external] = windows_desk();
        // 200x82 pt at 125 % is 250x102.5 px; the 8 pt margin is 10 px.
        assert_eq!(
            pill_origin(DesktopSpace::Pixels, laptop.work, laptop.scale, PILL),
            (1075.0, 1328.0)
        );
        assert_eq!(
            pill_origin(DesktopSpace::Pixels, external.work, external.scale, PILL),
            (3260.0, 950.0)
        );
    }

    #[test]
    fn mac_pill_lands_bottom_center_of_each_mixed_dpi_monitor() {
        let [retina, external] = mac_desk();
        assert_eq!(
            pill_origin(DesktopSpace::Points, retina.work, retina.scale, PILL),
            (620.0, 810.0)
        );
        assert_eq!(
            pill_origin(DesktopSpace::Points, external.work, external.scale, PILL),
            (2300.0, 990.0)
        );
    }

    #[test]
    fn pill_stays_inside_the_work_area_on_every_monitor() {
        for (space, desk) in [
            (DesktopSpace::Pixels, windows_desk()),
            (DesktopSpace::Points, mac_desk()),
        ] {
            for m in desk {
                let (x, y) = pill_origin(space, m.work, m.scale, PILL);
                // Back to the monitor's raw pixels to compare with its rect.
                let to_raw = if space == DesktopSpace::Points {
                    m.scale
                } else {
                    1.0
                };
                let (rx, ry) = (x * to_raw, y * to_raw);
                let (pw, ph) = (PILL.0 * m.scale, PILL.1 * m.scale);
                assert!(
                    rx >= m.work.x && rx + pw <= m.work.x + m.work.width,
                    "{space:?} x"
                );
                assert!(
                    ry >= m.work.y && ry + ph <= m.work.y + m.work.height,
                    "{space:?} y"
                );
            }
        }
    }

    #[test]
    fn tiny_work_area_pins_the_pill_to_its_corner() {
        let work = RawRect {
            x: 100.0,
            y: 100.0,
            width: 50.0,
            height: 50.0,
        };
        assert_eq!(
            pill_origin(DesktopSpace::Pixels, work, 1.0, PILL),
            (100.0, 100.0)
        );
    }

    #[test]
    fn windows_cursor_is_already_in_desktop_pixels() {
        // On the 100 % monitor while the primary is 125 %: dividing by the
        // primary scale would put the cursor back on the laptop.
        assert_eq!(
            cursor_in_desktop(DesktopSpace::Pixels, (3000.0, 500.0), 1.25),
            (3000.0, 500.0)
        );
    }

    #[test]
    fn mac_cursor_drops_the_primary_scale() {
        assert_eq!(
            cursor_in_desktop(DesktopSpace::Points, (3000.0, 500.0), 2.0),
            (1500.0, 250.0)
        );
    }

    #[test]
    fn windows_hit_test_scales_the_rect_by_the_window() {
        // Window at (1000, 1000) px on the 125 % laptop. The rect is 36x36 pt
        // at (82, 23) pt, so 45x45 px at (102.5, 28.75) px from the origin.
        let rect = [82.0, 23.0, 36.0, 36.0];
        let origin = (1000.0, 1000.0);
        assert!(cursor_over(
            DesktopSpace::Pixels,
            (1120.0, 1050.0),
            origin,
            1.25,
            rect,
            0.0
        ));
        // Inside in points but outside in pixels: the unscaled math would say yes.
        assert!(!cursor_over(
            DesktopSpace::Pixels,
            (1090.0, 1030.0),
            origin,
            1.25,
            rect,
            0.0
        ));
    }

    #[test]
    fn mac_hit_test_works_in_points() {
        let rect = [82.0, 23.0, 36.0, 36.0];
        // Window at 1000x500 pt on a 2x display: outer_position is 2000x1000.
        let origin = (2000.0, 1000.0);
        assert!(cursor_over(
            DesktopSpace::Points,
            (1100.0, 540.0),
            origin,
            2.0,
            rect,
            0.0
        ));
        assert!(!cursor_over(
            DesktopSpace::Points,
            (1070.0, 540.0),
            origin,
            2.0,
            rect,
            0.0
        ));
        // The margin widens the target.
        assert!(cursor_over(
            DesktopSpace::Points,
            (1078.0, 540.0),
            origin,
            2.0,
            rect,
            6.0
        ));
    }
}
