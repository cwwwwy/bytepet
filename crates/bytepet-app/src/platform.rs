/// Re-establish per-pixel transparency.
///
/// winit creates transparent windows by giving DWM an empty blur region, and
/// that is per-window state which a forced frame recompute can drop, so it is
/// re-applied after changing the window styles.
#[cfg(target_os = "windows")]
pub fn enable_transparency(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
    };
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as *mut core::ffi::c_void;
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let blur = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: 1,
            hRgnBlur: region,
            fTransitionOnMaximized: 0,
        };
        DwmEnableBlurBehindWindow(hwnd, &blur);
        DeleteObject(region);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn enable_transparency(_window: &winit::window::Window) {}

/// Remove every classic frame style from a window.
///
/// `decorations(false)` only hides the caption; the window still carries
/// `WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_MINIMIZEBOX |
/// WS_MAXIMIZEBOX`. Windows then redraws that non-client frame whenever the
/// window state changes (activation, a style change, a resize), which is the
/// border that flashed around the pet. A pure `WS_POPUP` window has no frame
/// to draw at all.
#[cfg(target_os = "windows")]
pub fn strip_frame_styles(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION, WS_DLGFRAME,
        WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    };

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as *mut core::ffi::c_void;
    let frame = WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let wanted = (style & !frame) | WS_POPUP;
        if style != wanted {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn strip_frame_styles(_window: &winit::window::Window) {}

/// Keep a window from ever becoming the active window.
///
/// Activating the pet window made Windows repaint its frame state (the flash
/// seen on the first drag) and stole focus from whatever the user was doing.
#[cfg(target_os = "windows")]
pub fn set_no_activate(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    };

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as *mut core::ffi::c_void;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_EX_NOACTIVATE == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE) as isize);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_no_activate(_window: &winit::window::Window) {}

/// Same as [`set_no_activate`] for a window identified by its title, used for
/// the menus that are created on demand. Returns how many windows were changed.
#[cfg(target_os = "windows")]
pub fn set_no_activate_for_title(title: &str) -> usize {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND,
    };
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongPtrW, GetWindowTextW, SetWindowLongPtrW, SetWindowPos,
        GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, WS_BORDER, WS_CAPTION, WS_DLGFRAME, WS_EX_NOACTIVATE, WS_MAXIMIZEBOX,
        WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    };

    struct Lookup<'a> {
        title: &'a str,
        hits: usize,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let lookup = unsafe { &mut *(lparam as *mut Lookup<'_>) };
        let mut buffer = [0u16; 256];
        let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        if len <= 0 {
            return TRUE;
        }
        let text = String::from_utf16_lossy(&buffer[..len as usize]);
        if text != lookup.title {
            return TRUE;
        }
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            if style & WS_EX_NOACTIVATE == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE) as isize);
            }
            // The popup windows are undecorated too, so give them the same
            // frame-free style as the pet.
            let frame =
                WS_CAPTION | WS_BORDER | WS_DLGFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
            let window_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let wanted = (window_style & !frame) | WS_POPUP;
            if window_style != wanted {
                SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
                // Keep the popup transparent after the frame recompute.
                let region = CreateRectRgn(0, 0, -1, -1);
                let blur = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: 1,
                    hRgnBlur: region,
                    fTransitionOnMaximized: 0,
                };
                DwmEnableBlurBehindWindow(hwnd, &blur);
                DeleteObject(region);
            }
        }
        lookup.hits += 1;
        TRUE
    }

    let mut lookup = Lookup { title, hits: 0 };
    unsafe {
        EnumWindows(Some(visit), &mut lookup as *mut _ as LPARAM);
    }
    lookup.hits
}

#[cfg(not(target_os = "windows"))]
pub fn set_no_activate_for_title(_title: &str) -> usize {
    0
}

/// Is Escape held? Menus do not take focus, so the key has to be polled.
#[cfg(target_os = "windows")]
pub fn escape_pressed() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};

    let state = unsafe { GetAsyncKeyState(VK_ESCAPE as i32) };
    (state as u16 & 0x8000) != 0
}

#[cfg(not(target_os = "windows"))]
pub fn escape_pressed() -> bool {
    false
}

/// Is the left mouse button held right now?
///
/// Dragging the pet is driven by the application instead of the OS modal move
/// loop, and that loop swallows the button-release event, so the state has to
/// come from the system. `None` means "not available on this platform".
#[cfg(target_os = "windows")]
pub fn primary_button_down() -> Option<bool> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};

    let state = unsafe { GetAsyncKeyState(VK_LBUTTON as i32) };
    Some((state as u16 & 0x8000) != 0)
}

#[cfg(not(target_os = "windows"))]
pub fn primary_button_down() -> Option<bool> {
    None
}

/// Is the right mouse button held right now?
#[cfg(target_os = "windows")]
pub fn secondary_button_down() -> Option<bool> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_RBUTTON};

    let state = unsafe { GetAsyncKeyState(VK_RBUTTON as i32) };
    Some((state as u16 & 0x8000) != 0)
}

#[cfg(not(target_os = "windows"))]
pub fn secondary_button_down() -> Option<bool> {
    None
}

/// Reveal a folder in the platform file manager.
///
/// Used by "打开宠物库目录" so the user can drop pet packages in by hand.
pub fn open_in_file_manager(path: &std::path::Path) -> bool {
    #[cfg(target_os = "windows")]
    let (program, args) = ("explorer.exe", vec![path.as_os_str().to_os_string()]);
    #[cfg(target_os = "macos")]
    let (program, args) = ("open", vec![path.as_os_str().to_os_string()]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, args) = ("xdg-open", vec![path.as_os_str().to_os_string()]);

    std::process::Command::new(program)
        .args(args)
        .spawn()
        .is_ok()
}

/// Best-effort global cursor position.
///
/// Windows is the important case for pixel-level click-through: once the
/// window has `WS_EX_TRANSPARENT`, egui may stop receiving cursor updates, so
/// we ask Win32 directly and can restore interaction when the cursor moves
/// back over an opaque pixel.
#[cfg(target_os = "windows")]
pub fn global_cursor_position() -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    let ok = unsafe { GetCursorPos(&mut point) };
    (ok != 0).then_some((point.x as f64, point.y as f64))
}

#[cfg(not(target_os = "windows"))]
pub fn global_cursor_position() -> Option<(f64, f64)> {
    None
}

#[cfg(target_os = "windows")]
pub fn clear_dwm_frame(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY,
    };

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as *mut core::ffi::c_void;
    let policy = DWMNCRP_DISABLED;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            &policy as *const _ as *const _,
            std::mem::size_of_val(&policy) as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub fn clear_dwm_frame(_window: &winit::window::Window) {}
