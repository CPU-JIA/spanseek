//! Read native restore bounds; persistence remains in the frontend's versioned session queue.
use pkms_app::WindowGeometry;
use tauri::WebviewWindow;

#[tauri::command]
pub fn read_window_restore_geometry(
    window: WebviewWindow,
) -> Result<Option<WindowGeometry>, &'static str> {
    if window.label() != "main" {
        return Err("window_geometry_forbidden");
    }
    #[cfg(windows)]
    {
        windows::read(&window).map(Some)
    }
    #[cfg(not(windows))]
    {
        // Other platforms retain the last observed normal rectangle while maximized.
        // Never mislabel maximized/minimized dimensions as normal restore bounds.
        if window
            .is_maximized()
            .map_err(|_| "window_geometry_read_failed")?
            || window
                .is_minimized()
                .map_err(|_| "window_geometry_read_failed")?
        {
            return Ok(None);
        }
        let size = window
            .inner_size()
            .map_err(|_| "window_geometry_read_failed")?;
        let position = window
            .outer_position()
            .map_err(|_| "window_geometry_read_failed")?;
        Ok(Some(WindowGeometry {
            width: size.width,
            height: size.height,
            left: position.x,
            top: position.y,
            maximized: false,
        }))
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, mem::size_of};

    // SDK 10.0.26100 WinUser.h: WINDOWPLACEMENT has no rcDevice on Windows (_MAC only).
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Placement {
        length: u32,
        flags: u32,
        show: u32,
        minimized: Point,
        maximized: Point,
        normal: Rect,
    }
    #[repr(C)]
    #[derive(Default)]
    struct MonitorInfo {
        length: u32,
        monitor: Rect,
        work: Rect,
        flags: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowPlacement(window: *mut c_void, placement: *mut Placement) -> i32;
        fn MonitorFromWindow(window: *mut c_void, flags: u32) -> *mut c_void;
        fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
        fn GetDpiForWindow(window: *mut c_void) -> u32;
        fn GetMenu(window: *mut c_void) -> *mut c_void;
        fn AdjustWindowRectExForDpi(
            rect: *mut Rect,
            style: u32,
            menu: i32,
            ex_style: u32,
            dpi: u32,
        ) -> i32;
        #[cfg(target_pointer_width = "64")]
        fn GetWindowLongPtrW(window: *mut c_void, index: i32) -> isize;
        #[cfg(target_pointer_width = "32")]
        fn GetWindowLongW(window: *mut c_void, index: i32) -> i32;
    }

    fn client_geometry(
        normal: Rect,
        frame: Rect,
        offset: Point,
        maximized: bool,
    ) -> Result<WindowGeometry, &'static str> {
        let width = i64::from(normal.right)
            - i64::from(normal.left)
            - (i64::from(frame.right) - i64::from(frame.left));
        let height = i64::from(normal.bottom)
            - i64::from(normal.top)
            - (i64::from(frame.bottom) - i64::from(frame.top));
        let left = i64::from(normal.left) + i64::from(offset.x);
        let top = i64::from(normal.top) + i64::from(offset.y);
        if !(640..=16000).contains(&width)
            || !(480..=16000).contains(&height)
            || left.abs() > 100000
            || top.abs() > 100000
        {
            return Err("window_geometry_out_of_bounds");
        }
        Ok(WindowGeometry {
            width: width as u32,
            height: height as u32,
            left: left as i32,
            top: top as i32,
            maximized,
        })
    }

    pub(super) fn read(window: &WebviewWindow) -> Result<WindowGeometry, &'static str> {
        let hwnd = window
            .hwnd()
            .map_err(|_| "window_geometry_handle_failed")?
            .0;
        let mut placement = Placement {
            length: size_of::<Placement>() as u32,
            ..Default::default()
        };
        let mut monitor = MonitorInfo {
            length: size_of::<MonitorInfo>() as u32,
            ..Default::default()
        };
        // The handle comes from the caller's verified main window. All calls are read-only;
        // stack buffers have the exact Win32 ABI and initialized length members.
        let (style, ex_style, dpi, menu) = unsafe {
            if GetWindowPlacement(hwnd, &mut placement) == 0 {
                return Err("window_geometry_placement_failed");
            }
            let handle = MonitorFromWindow(hwnd, 2); // MONITOR_DEFAULTTONEAREST
            if handle.is_null() || GetMonitorInfoW(handle, &mut monitor) == 0 {
                return Err("window_geometry_monitor_failed");
            }
            #[cfg(target_pointer_width = "64")]
            let styles = (
                GetWindowLongPtrW(hwnd, -16) as u32,
                GetWindowLongPtrW(hwnd, -20) as u32,
            );
            #[cfg(target_pointer_width = "32")]
            let styles = (
                GetWindowLongW(hwnd, -16) as u32,
                GetWindowLongW(hwnd, -20) as u32,
            );
            (
                styles.0,
                styles.1,
                GetDpiForWindow(hwnd),
                !GetMenu(hwnd).is_null(),
            )
        };
        if dpi == 0 || style == 0 {
            return Err("window_geometry_style_failed");
        }
        let mut frame = Rect::default();
        // Remove WS_MAXIMIZE/WS_MINIMIZE before measuring the normal non-client frame.
        // This queries the window's actual DPI and never changes thread/system DPI.
        if unsafe {
            AdjustWindowRectExForDpi(
                &mut frame,
                style & !(0x01000000 | 0x20000000),
                i32::from(menu),
                ex_style,
                dpi,
            )
        } == 0
        {
            return Err("window_geometry_frame_failed");
        }
        // Ordinary top-level WINDOWPLACEMENT coordinates exclude taskbar/toolbars.
        // Tauri setPosition uses screen coordinates. Tool windows already use screen coordinates.
        let offset = if ex_style & 0x00000080 == 0 {
            Point {
                x: monitor.work.left - monitor.monitor.left,
                y: monitor.work.top - monitor.monitor.top,
            }
        } else {
            Point::default()
        };
        client_geometry(placement.normal, frame, offset, placement.show == 3)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn win32_layout_and_taskbar_coordinates_preserve_exact_client_pixels() {
            assert_eq!(size_of::<Placement>(), 44);
            assert_eq!(size_of::<MonitorInfo>(), 40);
            let value = client_geometry(
                Rect {
                    left: -1800,
                    top: -60,
                    right: -559,
                    bottom: 801,
                },
                Rect {
                    left: -12,
                    top: -47,
                    right: 12,
                    bottom: 12,
                },
                Point { x: 40, y: 32 },
                true,
            )
            .unwrap();
            assert_eq!(
                (
                    value.width,
                    value.height,
                    value.left,
                    value.top,
                    value.maximized
                ),
                (1217, 802, -1760, -28, true)
            );
        }
        #[test]
        fn malformed_or_unrepresentable_native_bounds_are_rejected() {
            assert!(
                client_geometry(Rect::default(), Rect::default(), Point::default(), false).is_err()
            );
            assert!(
                client_geometry(
                    Rect {
                        left: i32::MIN,
                        top: 0,
                        right: i32::MAX,
                        bottom: 900
                    },
                    Rect::default(),
                    Point::default(),
                    false
                )
                .is_err()
            );
        }
    }
}
