use std::sync::Mutex;

use tauri::{LogicalSize, PhysicalSize, WebviewWindow};

use crate::models::WindowSize;

/// Keep these values synchronized with the native constraints in `tauri.conf.json`.
pub const MIN_WINDOW_WIDTH: u32 = 900;
pub const MIN_WINDOW_HEIGHT: u32 = 640;
const MAX_PERSISTED_DIMENSION: u32 = 32_768;

#[derive(Debug, Default)]
pub struct WindowSizeMemory {
    last_normal_size: Mutex<Option<WindowSize>>,
}

impl WindowSizeMemory {
    pub fn new(initial: Option<WindowSize>) -> Self {
        Self {
            last_normal_size: Mutex::new(initial),
        }
    }

    pub fn remember(&self, size: WindowSize) {
        *self
            .last_normal_size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(size);
    }

    pub fn get(&self) -> Option<WindowSize> {
        *self
            .last_normal_size
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Reject corrupt values before they can influence native window creation. A valid size from a
/// larger previous monitor remains eligible and is clamped to the current monitor on restore.
pub fn validate_persisted_size(size: WindowSize) -> Option<WindowSize> {
    (size.width >= MIN_WINDOW_WIDTH
        && size.height >= MIN_WINDOW_HEIGHT
        && size.width <= MAX_PERSISTED_DIMENSION
        && size.height <= MAX_PERSISTED_DIMENSION)
        .then_some(size)
}

pub fn logical_size_from_physical(
    size: PhysicalSize<u32>,
    scale_factor: f64,
) -> Option<WindowSize> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let width = (f64::from(size.width) / scale_factor).round();
    let height = (f64::from(size.height) / scale_factor).round();
    if !(0.0..=f64::from(u32::MAX)).contains(&width)
        || !(0.0..=f64::from(u32::MAX)).contains(&height)
    {
        return None;
    }
    validate_persisted_size(WindowSize {
        width: width as u32,
        height: height as u32,
    })
}

pub fn should_capture_window_size(minimized: bool, maximized: bool, fullscreen: bool) -> bool {
    !minimized && !maximized && !fullscreen
}

/// Clamp a valid saved logical size to the current monitor work area. The minimum remains the
/// configured Tauri minimum even on an unusually small work area because the native constraint is
/// authoritative and cannot be safely bypassed here.
pub fn clamp_to_work_area(size: WindowSize, work_area: Option<(u32, u32)>) -> WindowSize {
    let Some((available_width, available_height)) = work_area else {
        return size;
    };
    WindowSize {
        width: size
            .width
            .min(available_width.max(MIN_WINDOW_WIDTH))
            .max(MIN_WINDOW_WIDTH),
        height: size
            .height
            .min(available_height.max(MIN_WINDOW_HEIGHT))
            .max(MIN_WINDOW_HEIGHT),
    }
}

/// Resolve a persisted logical size before the hidden native window is shown. Keeping validation
/// and work-area clamping in this pure step makes the first visible frame obey the same safety
/// contract as later window events.
pub fn resolve_startup_size(
    saved: WindowSize,
    work_area: Option<(u32, u32)>,
) -> Option<WindowSize> {
    validate_persisted_size(saved).map(|size| clamp_to_work_area(size, work_area))
}

fn monitor_work_area_logical(window: &WebviewWindow) -> Option<(u32, u32)> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())?;
    let scale_factor = monitor.scale_factor();
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let physical = monitor.work_area().size;
    Some((
        (f64::from(physical.width) / scale_factor).floor() as u32,
        (f64::from(physical.height) / scale_factor).floor() as u32,
    ))
}

pub fn restore_window_size(window: &WebviewWindow, saved: WindowSize) -> Option<WindowSize> {
    let restored = resolve_startup_size(saved, monitor_work_area_logical(window))?;
    window
        .set_size(LogicalSize::new(
            f64::from(restored.width),
            f64::from(restored.height),
        ))
        .ok()?;
    // The initial config is centered; resizing after window creation needs a second best-effort
    // center operation so a larger remembered window cannot start partially off screen.
    let _ = window.center();
    Some(restored)
}

pub fn capture_normal_window_size(
    window: &WebviewWindow,
    physical_size: PhysicalSize<u32>,
    scale_factor: Option<f64>,
) -> Option<WindowSize> {
    let minimized = window.is_minimized().ok()?;
    let maximized = window.is_maximized().ok()?;
    let fullscreen = window.is_fullscreen().ok()?;
    if !should_capture_window_size(minimized, maximized, fullscreen) {
        return None;
    }
    logical_size_from_physical(
        physical_size,
        scale_factor.or_else(|| window.scale_factor().ok())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_physical_dimensions_to_logical_across_dpi() {
        assert_eq!(
            logical_size_from_physical(
                PhysicalSize {
                    width: 2_560,
                    height: 1_600,
                },
                2.0,
            ),
            Some(WindowSize {
                width: 1_280,
                height: 800,
            })
        );
        assert_eq!(
            logical_size_from_physical(
                PhysicalSize {
                    width: 1_280,
                    height: 800,
                },
                0.0,
            ),
            None
        );
    }

    #[test]
    fn rejects_corrupt_or_below_minimum_persisted_sizes() {
        assert_eq!(
            validate_persisted_size(WindowSize {
                width: 899,
                height: 640,
            }),
            None
        );
        assert_eq!(
            validate_persisted_size(WindowSize {
                width: 900,
                height: 639,
            }),
            None
        );
        assert_eq!(
            validate_persisted_size(WindowSize {
                width: MAX_PERSISTED_DIMENSION + 1,
                height: 800,
            }),
            None
        );
    }

    #[test]
    fn clamps_a_previous_monitor_size_to_the_current_work_area() {
        assert_eq!(
            clamp_to_work_area(
                WindowSize {
                    width: 3_840,
                    height: 2_160,
                },
                Some((1_920, 1_040)),
            ),
            WindowSize {
                width: 1_920,
                height: 1_040,
            }
        );
        assert_eq!(
            clamp_to_work_area(
                WindowSize {
                    width: 1_280,
                    height: 800,
                },
                Some((800, 600)),
            ),
            WindowSize {
                width: MIN_WINDOW_WIDTH,
                height: MIN_WINDOW_HEIGHT,
            }
        );
    }

    #[test]
    fn resolves_only_safe_startup_sizes_before_first_show() {
        assert_eq!(
            resolve_startup_size(
                WindowSize {
                    width: 2_560,
                    height: 1_440,
                },
                Some((1_920, 1_040)),
            ),
            Some(WindowSize {
                width: 1_920,
                height: 1_040,
            })
        );
        assert_eq!(
            resolve_startup_size(
                WindowSize {
                    width: 640,
                    height: 480,
                },
                Some((1_920, 1_040)),
            ),
            None
        );
    }

    #[test]
    fn only_normal_windows_are_capture_candidates() {
        assert!(should_capture_window_size(false, false, false));
        assert!(!should_capture_window_size(true, false, false));
        assert!(!should_capture_window_size(false, true, false));
        assert!(!should_capture_window_size(false, false, true));
    }
}
