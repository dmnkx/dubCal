//! Embed the calendar above the desktop icon surface, below ordinary windows.
//! Explorer's SHELLDLL_DefView is an implementation detail, so attachment can fail.
use crate::geometry::Geometry;

#[cfg(windows)]
mod platform {
    use super::Geometry;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ptr::null_mut;
    use windows_sys::{
        core::BOOL,
        Win32::{
            Foundation::{GetLastError, SetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
            Graphics::Gdi::{
                GetMonitorInfoW, MonitorFromPoint, RedrawWindow, ScreenToClient, MONITORINFO,
                MONITOR_DEFAULTTONEAREST, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE,
            },
            UI::{
                Input::KeyboardAndMouse::{GetFocus, SetFocus},
                Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
                WindowsAndMessaging::*,
            },
        },
    };

    #[derive(Default)]
    pub struct Desktop {
        window: isize,
        parent: isize,
        attempted: bool,
        error: Option<String>,
        grab: Option<POINT>,
        opacity: Option<u8>,
        opacity_error: Option<String>,
        resize_grab: Option<(POINT, Geometry)>,
        minimum: [i32; 2],
        snap_distance: i32,
    }

    unsafe extern "system" fn find_desktop(window: HWND, data: LPARAM) -> BOOL {
        let class: Vec<u16> = "SHELLDLL_DefView\0".encode_utf16().collect();
        let view = FindWindowExW(window, null_mut(), class.as_ptr(), std::ptr::null());
        if !view.is_null() {
            *(data as *mut HWND) = view;
            return 0;
        }
        1
    }

    impl Desktop {
        pub fn attach(&mut self, frame: &eframe::Frame, saved: Option<Geometry>, scale: f32) {
            self.snap_distance = (16.0 * scale).round().max(1.0) as i32;
            self.minimum = [
                (700.0 * scale).round() as i32,
                (500.0 * scale).round() as i32,
            ];
            if self.attempted {
                return;
            }
            self.attempted = true;
            if let Err(error) = self.try_attach(frame, saved) {
                self.error = Some(format!("바탕화면 위젯 연결 실패: {error}"));
            }
        }

        fn try_attach(
            &mut self,
            frame: &eframe::Frame,
            saved: Option<Geometry>,
        ) -> anyhow::Result<()> {
            let RawWindowHandle::Win32(handle) = frame.window_handle()?.as_raw() else {
                anyhow::bail!("Windows 창 핸들을 찾을 수 없습니다");
            };
            let window = handle.hwnd.get() as HWND;
            // Run on the UI thread. Keep all pointers local to the Win32 calls.
            unsafe {
                let mut parent: HWND = null_mut();
                EnumWindows(Some(find_desktop), &mut parent as *mut HWND as LPARAM);
                anyhow::ensure!(!parent.is_null(), "Explorer 바탕화면을 찾을 수 없습니다");
                let style = GetWindowLongPtrW(window, GWL_STYLE);
                let ex_style = GetWindowLongPtrW(window, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    window,
                    GWL_STYLE,
                    (style & !(WS_POPUP as isize | WS_CAPTION as isize | WS_THICKFRAME as isize))
                        | WS_CHILD as isize,
                );
                SetLastError(0);
                let old_parent = SetParent(window, parent);
                let error = GetLastError();
                if old_parent.is_null() && error != 0 {
                    SetWindowLongPtrW(window, GWL_STYLE, style);
                    anyhow::bail!(
                        "SetParent: {}",
                        std::io::Error::from_raw_os_error(error as i32)
                    );
                }
                SetWindowLongPtrW(
                    window,
                    GWL_EXSTYLE,
                    (ex_style & !((WS_EX_APPWINDOW | WS_EX_NOACTIVATE) as isize))
                        | WS_EX_TOOLWINDOW as isize,
                );
                let mut rect = RECT::default();
                anyhow::ensure!(
                    GetWindowRect(window, &mut rect) != 0,
                    "위젯 크기를 읽을 수 없습니다"
                );
                let desired = saved.unwrap_or(Geometry {
                    x: 40,
                    y: 60,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                });
                let geometry = fitted_geometry(desired, self.minimum);
                let position = parent_position(parent, geometry);
                if SetWindowPos(
                    window,
                    HWND_TOP,
                    position.x,
                    position.y,
                    geometry.width,
                    geometry.height,
                    SWP_NOACTIVATE | SWP_FRAMECHANGED,
                ) == 0
                {
                    let error = std::io::Error::last_os_error();
                    SetParent(window, old_parent);
                    SetWindowLongPtrW(window, GWL_STYLE, style);
                    SetWindowLongPtrW(window, GWL_EXSTYLE, ex_style);
                    anyhow::bail!("위젯 배치: {error}");
                }
                self.window = window as isize;
                self.parent = parent as isize;
                self.error = None;
                self.opacity = None;
            }
            Ok(())
        }

        pub fn error(&self) -> Option<&str> {
            self.error.as_deref()
        }
        pub fn retry(&mut self) {
            self.attempted = false;
        }

        pub fn opacity_error(&self) -> Option<&str> {
            self.opacity_error.as_deref()
        }

        pub fn set_opacity(&mut self, alpha: u8) {
            if self.window == 0 {
                return;
            }
            unsafe {
                let window = self.window as HWND;
                // Check the HWND, not just our cached setting: winit can rewrite styles
                // after App::update, notably when it shows the first rendered frame.
                if self.opacity == Some(alpha)
                    && (self.opacity_error.is_some() || opacity_matches(window, alpha))
                {
                    return;
                }
                self.opacity = Some(alpha);
                self.opacity_error = None;
                if SetWindowSubclass(
                    window,
                    Some(widget_style_proc),
                    STYLE_SUBCLASS,
                    alpha as usize,
                ) == 0
                {
                    self.opacity_error = Some("투명도 초기화 보호를 설치하지 못했습니다".into());
                    return;
                }
                let previous = GetWindowLongPtrW(window, GWL_EXSTYLE);
                let style = if alpha == 255 {
                    previous & !(WS_EX_LAYERED as isize)
                } else {
                    previous | WS_EX_LAYERED as isize
                };
                SetLastError(0);
                let old = SetWindowLongPtrW(window, GWL_EXSTYLE, style);
                if old == 0 && GetLastError() != 0 {
                    let error = std::io::Error::last_os_error();
                    RemoveWindowSubclass(window, Some(widget_style_proc), STYLE_SUBCLASS);
                    self.opacity_error = Some(format!("투명도를 적용하지 못했습니다: {error}"));
                    return;
                }
                if alpha != 255 && SetLayeredWindowAttributes(window, 0, alpha, LWA_ALPHA) == 0 {
                    let error = std::io::Error::last_os_error();
                    RemoveWindowSubclass(window, Some(widget_style_proc), STYLE_SUBCLASS);
                    SetWindowLongPtrW(window, GWL_EXSTYLE, previous);
                    self.opacity_error = Some(format!("투명도를 적용하지 못했습니다: {error}"));
                }
                RedrawWindow(
                    window,
                    std::ptr::null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                );
            }
        }

        pub fn geometry(&self) -> Option<Geometry> {
            if self.window == 0 {
                return None;
            }
            unsafe {
                let mut rect = RECT::default();
                if GetWindowRect(self.window as HWND, &mut rect) == 0 {
                    return None;
                }
                Some(Geometry {
                    x: rect.left,
                    y: rect.top,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                })
            }
        }

        // Keep the full widget in the monitor's work area after display changes.
        pub fn keep_visible(&self) {
            if let Some(current) = self.geometry() {
                unsafe {
                    self.place(fitted_geometry(current, self.minimum));
                }
            }
        }

        unsafe fn place(&self, geometry: Geometry) {
            if self.geometry() == Some(geometry) {
                return;
            }
            let position = parent_position(self.parent as HWND, geometry);
            SetWindowPos(
                self.window as HWND,
                HWND_TOP,
                position.x,
                position.y,
                geometry.width,
                geometry.height,
                SWP_NOACTIVATE,
            );
        }

        pub fn resize(&mut self, resizing: bool) {
            if !resizing || self.window == 0 {
                self.resize_grab = None;
                return;
            }
            unsafe {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor) == 0 {
                    return;
                }
                let Some(current) = self.geometry() else {
                    return;
                };
                let (start, geometry) = *self.resize_grab.get_or_insert((cursor, current));
                let desired = Geometry {
                    width: geometry
                        .width
                        .saturating_add(cursor.x.saturating_sub(start.x)),
                    height: geometry
                        .height
                        .saturating_add(cursor.y.saturating_sub(start.y)),
                    ..geometry
                };
                self.place(fitted_geometry(desired, self.minimum));
            }
        }

        pub fn drag(&mut self, dragging: bool) {
            if !dragging || self.window == 0 {
                self.grab = None;
                return;
            }
            unsafe {
                let window = self.window as HWND;
                let parent = self.parent as HWND;
                let mut cursor = POINT::default();
                let mut rect = RECT::default();
                if GetCursorPos(&mut cursor) == 0 || GetWindowRect(window, &mut rect) == 0 {
                    return;
                }
                let grab = *self.grab.get_or_insert(POINT {
                    x: cursor.x - rect.left,
                    y: cursor.y - rect.top,
                });
                let geometry = fitted_geometry(
                    Geometry {
                        x: cursor.x - grab.x,
                        y: cursor.y - grab.y,
                        width: rect.right - rect.left,
                        height: rect.bottom - rect.top,
                    },
                    self.minimum,
                );
                let geometry = if let Some(work) = monitor_work_area(geometry) {
                    geometry.snap(work, self.snap_distance)
                } else {
                    geometry
                };
                let position = parent_position(parent, geometry);
                SetWindowPos(
                    window,
                    HWND_TOP,
                    position.x,
                    position.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    const STYLE_SUBCLASS: usize = 0xD35C;

    /// Protect native desktop/opacity styles when winit calls set_visible(true).
    /// Only a numeric alpha is stored in ref_data; there is no borrowed Rust state.
    unsafe extern "system" fn widget_style_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass: usize,
        ref_data: usize,
    ) -> LRESULT {
        let alpha = ref_data as u8;
        if message == WM_MOUSEACTIVATE {
            // DefWindowProc would delegate activation to Explorer. The desktop
            // can refuse it, leaving clickable egui inputs without keyboard focus.
            // Do not eat the click: the input must receive the first click too.
            return MA_ACTIVATE as LRESULT;
        }
        if matches!(
            message,
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        ) {
            // Only a deliberate click may focus this child; repaint/move/resize
            // must never take keyboard focus from another application.
            if GetFocus() != window {
                SetFocus(window);
            }
        }
        if message == WM_SETFOCUS {
            // winit 0.30 emits Focused(true) only if both its active and focused
            // flags are set. A child of Explorer gets WM_SETFOCUS but normally
            // has no non-client activation of its own. Notify the original winit
            // procedure, then forward real focus normally. Do not intercept key
            // or IME messages: winit/egui retain their normal text-input path.
            DefSubclassProc(window, WM_NCACTIVATE, 1, -1);
            return DefSubclassProc(window, message, wparam, lparam);
        }
        if message == WM_KILLFOCUS {
            let result = DefSubclassProc(window, message, wparam, lparam);
            DefSubclassProc(window, WM_NCACTIVATE, 0, -1);
            return result;
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(window, Some(widget_style_proc), subclass);
            return DefSubclassProc(window, message, wparam, lparam);
        }
        if message == WM_STYLECHANGING && lparam != 0 {
            // Allow the underlying window procedure to edit the proposed style first.
            let result = DefSubclassProc(window, message, wparam, lparam);
            let styles = &mut *(lparam as *mut STYLESTRUCT);
            if wparam as i32 == GWL_EXSTYLE {
                styles.styleNew =
                    (styles.styleNew & !(WS_EX_APPWINDOW | WS_EX_NOACTIVATE)) | WS_EX_TOOLWINDOW;
                if alpha == 255 {
                    styles.styleNew &= !WS_EX_LAYERED;
                } else {
                    styles.styleNew |= WS_EX_LAYERED;
                }
            } else if wparam as i32 == GWL_STYLE {
                styles.styleNew =
                    (styles.styleNew & !(WS_POPUP | WS_CAPTION | WS_THICKFRAME)) | WS_CHILD;
            }
            return result;
        }
        let result = DefSubclassProc(window, message, wparam, lparam);
        if message == WM_STYLECHANGED && wparam as i32 == GWL_EXSTYLE && alpha != 255 {
            SetLayeredWindowAttributes(window, 0, alpha, LWA_ALPHA);
        }
        result
    }

    unsafe fn opacity_matches(window: HWND, alpha: u8) -> bool {
        let layered = GetWindowLongPtrW(window, GWL_EXSTYLE) & WS_EX_LAYERED as isize != 0;
        if alpha == 255 {
            return !layered;
        }
        if !layered {
            return false;
        }
        let mut actual = 0;
        let mut flags = 0;
        GetLayeredWindowAttributes(window, std::ptr::null_mut(), &mut actual, &mut flags) != 0
            && flags & LWA_ALPHA != 0
            && actual == alpha
    }

    impl Drop for Desktop {
        fn drop(&mut self) {
            // All use is on the UI thread. WM_NCDESTROY also removes the callback.
            if self.window != 0 {
                unsafe {
                    RemoveWindowSubclass(
                        self.window as HWND,
                        Some(widget_style_proc),
                        STYLE_SUBCLASS,
                    );
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn clicking_desktop_child_focuses_it_and_unicode_text_is_forwarded() {
            struct Host(HWND);
            impl Drop for Host {
                fn drop(&mut self) {
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            unsafe {
                let host_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                let edit_class: Vec<u16> = "EDIT\0".encode_utf16().collect();
                let host = Host(CreateWindowExW(
                    0,
                    host_class.as_ptr(),
                    std::ptr::null(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    800,
                    600,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                ));
                assert!(!host.0.is_null());
                let child = CreateWindowExW(
                    0,
                    edit_class.as_ptr(),
                    std::ptr::null(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    300,
                    100,
                    host.0,
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                );
                assert!(!child.is_null());
                let mut desktop = Desktop::default();
                desktop.window = child as isize;
                desktop.parent = host.0 as isize;
                desktop.set_opacity(255);
                assert!(desktop.opacity_error().is_none());
                ShowWindow(host.0, SW_SHOWNOACTIVATE);
                SetFocus(host.0);
                assert_eq!(
                    SendMessageW(
                        child,
                        WM_MOUSEACTIVATE,
                        host.0 as WPARAM,
                        ((WM_LBUTTONDOWN << 16) | HTCLIENT) as LPARAM
                    ),
                    MA_ACTIVATE as LRESULT
                );
                SendMessageW(child, WM_LBUTTONDOWN, 0, 0);
                assert_eq!(GetFocus(), child);
                SendMessageW(child, WM_LBUTTONUP, 0, 0);
                for character in "한글ABC".encode_utf16() {
                    SendMessageW(child, WM_CHAR, character as WPARAM, 0);
                }
                let mut text = [0u16; 32];
                let length = GetWindowTextW(child, text.as_mut_ptr(), text.len() as i32);
                assert_eq!(
                    String::from_utf16_lossy(&text[..length as usize]),
                    "한글ABC"
                );
                SetFocus(host.0);
                assert_eq!(GetFocus(), host.0);
            }
        }

        #[test]
        fn saved_opacity_survives_first_show_style_reset() {
            // Emulate eframe/winit's first-show rewrite on real HWNDs. The host
            // stands in for Explorer; neither window needs to be visible.
            struct Host(HWND);
            impl Drop for Host {
                fn drop(&mut self) {
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            unsafe {
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                let host = Host(CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    1000,
                    800,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                ));
                assert!(!host.0.is_null(), "{}", std::io::Error::last_os_error());
                let child = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_CHILD,
                    0,
                    0,
                    200,
                    200,
                    host.0,
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                );
                assert!(!child.is_null(), "{}", std::io::Error::last_os_error());
                let mut desktop = Desktop::default();
                desktop.window = child as isize;
                desktop.parent = host.0 as isize;
                desktop.set_opacity(102); // Restored transparency: 60%.
                assert!(
                    desktop.opacity_error().is_none(),
                    "{:?}",
                    desktop.opacity_error()
                );
                assert!(opacity_matches(child, 102));

                // This previously removed WS_EX_LAYERED after the first frame.
                SetWindowLongPtrW(child, GWL_STYLE, WS_POPUP as isize);
                SetWindowLongPtrW(child, GWL_EXSTYLE, WS_EX_APPWINDOW as isize);
                assert!(opacity_matches(child, 102));
                assert_ne!(GetWindowLongPtrW(child, GWL_STYLE) & WS_CHILD as isize, 0);
                assert_eq!(
                    GetWindowLongPtrW(child, GWL_EXSTYLE) & WS_EX_APPWINDOW as isize,
                    0
                );

                // The new value must replace the subclass's reference data too.
                desktop.set_opacity(51);
                SetWindowLongPtrW(child, GWL_EXSTYLE, 0);
                assert!(opacity_matches(child, 51));
                desktop.set_opacity(255);
                SetWindowLongPtrW(child, GWL_EXSTYLE, WS_EX_LAYERED as isize);
                assert!(opacity_matches(child, 255));
            }
        }
    }

    unsafe fn parent_position(parent: HWND, geometry: Geometry) -> POINT {
        let mut position = POINT {
            x: geometry.x,
            y: geometry.y,
        };
        ScreenToClient(parent, &mut position);
        position
    }

    unsafe fn monitor_work_area(geometry: Geometry) -> Option<Geometry> {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor = MonitorFromPoint(
            POINT {
                x: geometry.x,
                y: geometry.y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        Some(Geometry {
            x: work.left,
            y: work.top,
            width: work.right - work.left,
            height: work.bottom - work.top,
        })
    }

    unsafe fn fitted_geometry(geometry: Geometry, minimum: [i32; 2]) -> Geometry {
        monitor_work_area(geometry).map_or(geometry, |work| geometry.fit(work, minimum))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Geometry;
    #[derive(Default)]
    pub struct Desktop;
    impl Desktop {
        pub fn attach(&mut self, _: &eframe::Frame, _: Option<Geometry>, _: f32) {}
        pub fn error(&self) -> Option<&str> {
            Some("바탕화면 위젯은 Windows에서 지원됩니다")
        }
        pub fn retry(&mut self) {}
        pub fn drag(&mut self, _: bool) {}
        pub fn keep_visible(&self) {}
        pub fn geometry(&self) -> Option<Geometry> {
            None
        }
        pub fn resize(&mut self, _: bool) {}
        pub fn set_opacity(&mut self, _: u8) {}
        pub fn opacity_error(&self) -> Option<&str> {
            None
        }
    }
}

pub use platform::Desktop;
