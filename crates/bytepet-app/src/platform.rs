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
