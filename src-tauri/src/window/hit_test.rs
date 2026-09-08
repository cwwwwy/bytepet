//! Per-pixel click-through.
//!
//! Tauri can make a whole window transparent to the mouse, but a pet must stay
//! clickable only where it is actually drawn. We therefore poll the global
//! cursor position, map it into the current animation cell and consult the
//! downsampled alpha mask, toggling `set_ignore_cursor_events` only when the
//! result changes.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use std::time::Duration;

use parking_lot::Mutex;
use bytepet_core::config::ClickThroughMode;
use bytepet_core::pet::atlas::AlphaMask;
use bytepet_core::pet::manifest::FrameSpec;
use tauri::{AppHandle, Manager};

/// Cursor polling interval. 30 Hz is invisible to the user and cheap.
const POLL_INTERVAL: Duration = Duration::from_millis(33);

/// Shared hit-test state. The renderer updates `sprite`; the active pet updates
/// `mask`/`frame`; settings update `mode`.
#[derive(Default)]
pub struct HitState {
    /// Global sprite index currently drawn.
    pub sprite: AtomicU32,
    /// Mask of the active spritesheet.
    pub mask: Mutex<Option<AlphaMask>>,
    pub frame: Mutex<FrameSpec>,
    pub mode: Mutex<ClickThroughMode>,
    /// Window logical size / cell size, so cursor coordinates can be mapped
    /// back into cell pixels.
    pub scale: Mutex<f32>,
    /// When false the window behaves as fully interactive (used while dragging).
    pub suspended: AtomicBool,
    /// Safety net: dragging must never leave the window permanently interactive
    /// if the pointerup event is lost to the OS drag loop.
    pub suspended_until: Mutex<Option<std::time::Instant>>,
    /// Last applied ignore-cursor-events value, to avoid redundant IPC.
    last_ignored: AtomicBool,
}

impl HitState {
    pub fn set_atlas(&self, mask: AlphaMask, frame: FrameSpec) {
        *self.mask.lock() = Some(mask);
        *self.frame.lock() = frame;
    }

    pub fn set_scale(&self, scale: f32) {
        *self.scale.lock() = scale.max(0.05);
    }

    pub fn set_mode(&self, mode: ClickThroughMode) {
        *self.mode.lock() = mode;
    }

    /// Suspend/resume hit testing while the user drags the pet.
    pub fn set_suspended(&self, suspended: bool) {
        self.suspended.store(suspended, Ordering::Relaxed);
        *self.suspended_until.lock() = if suspended {
            Some(std::time::Instant::now() + Duration::from_secs(5))
        } else {
            None
        };
    }

    /// True while a drag is in progress and its safety deadline has not passed.
    pub fn is_suspended(&self) -> bool {
        if !self.suspended.load(Ordering::Relaxed) {
            return false;
        }
        let expired = self
            .suspended_until
            .lock()
            .is_some_and(|deadline| std::time::Instant::now() >= deadline);
        if expired {
            self.set_suspended(false);
            return false;
        }
        true
    }

    /// Should the window ignore mouse events for the given window-local point?
    ///
    /// `x`/`y` are window logical pixels; they are divided by the display scale
    /// to become cell pixels.
    pub fn should_ignore_window_point(&self, x: f32, y: f32) -> bool {
        let scale = *self.scale.lock();
        self.should_ignore(x / scale, y / scale)
    }

    /// Should the window ignore mouse events for the given window-local point?
    pub fn should_ignore(&self, x: f32, y: f32) -> bool {
        let frame = *self.frame.lock();
        if x < 0.0 || y < 0.0 || x >= frame.width as f32 || y >= frame.height as f32 {
            return true;
        }
        match *self.mode.lock() {
            ClickThroughMode::Passthrough => true,
            ClickThroughMode::Rect => false,
            ClickThroughMode::Auto => match self.mask.lock().as_ref() {
                Some(mask) => !mask.opaque_at_cell(self.sprite.load(Ordering::Relaxed), x, y),
                None => false,
            },
        }
    }

    /// Apply the result to the window, returning the value that was set.
    pub fn apply(&self, app: &AppHandle, ignore: bool) -> bool {
        if self.last_ignored.load(Ordering::Relaxed) == ignore {
            return ignore;
        }
        if let Some(win) = app.get_webview_window("pet") {
            if win.set_ignore_cursor_events(ignore).is_ok() {
                self.last_ignored.store(ignore, Ordering::Relaxed);
            }
        }
        ignore
    }
}

/// Start the polling loop. It re-reads the active runtime each tick, so
/// switching pets does not accumulate threads. Returns immediately.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL_INTERVAL);
        let Some(state) = app.try_state::<crate::state::AppState>() else {
            continue;
        };
        let Some(runtime) = state.pet_runtime() else {
            continue;
        };
        let hit = runtime.hit.clone();
        let Some(win) = app.get_webview_window("pet") else {
            continue;
        };
        if !win.is_visible().unwrap_or(false) {
            continue;
        }

        let Ok(cursor) = app.cursor_position() else {
            continue;
        };
        let Ok(pos) = win.outer_position() else {
            continue;
        };
        let Ok(scale) = win.scale_factor() else {
            continue;
        };
        let Ok(size) = win.inner_size() else {
            continue;
        };

        // Cursor and window position are physical pixels; convert to
        // window-local logical pixels.
        let rel_x = (cursor.x - pos.x as f64) / scale;
        let rel_y = (cursor.y - pos.y as f64) / scale;
        let inside = rel_x >= 0.0
            && rel_y >= 0.0
            && rel_x < (size.width as f64) / scale
            && rel_y < (size.height as f64) / scale;

        let ignore = if hit.is_suspended() {
            false
        } else if !inside {
            true
        } else {
            hit.should_ignore_window_point(rel_x as f32, rel_y as f32)
        };
        hit.apply(&app, ignore);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn state_with_square() -> HitState {
        let frame = FrameSpec::new(2, 1);
        let mut img = RgbaImage::new(frame.atlas_width(), frame.atlas_height());
        for y in 0..20 {
            for x in 0..20 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let atlas = bytepet_core::pet::PetAtlas::from_image(img, frame, "t".into()).unwrap();
        let state = HitState::default();
        state.set_atlas(atlas.mask, frame);
        state
    }

    #[test]
    fn auto_mode_hits_only_drawn_pixels() {
        let state = state_with_square();
        state.sprite.store(0, Ordering::Relaxed);
        assert!(!state.should_ignore(5.0, 5.0));
        assert!(state.should_ignore(100.0, 100.0));
        // Cell 1 is empty.
        state.sprite.store(1, Ordering::Relaxed);
        assert!(state.should_ignore(5.0, 5.0));
        // Outside the cell grid entirely.
        assert!(state.should_ignore(-1.0, 0.0));
        assert!(state.should_ignore(0.0, 999.0));
    }

    #[test]
    fn rect_and_passthrough_modes() {
        let state = state_with_square();
        state.set_mode(ClickThroughMode::Rect);
        assert!(!state.should_ignore(100.0, 100.0));
        state.set_mode(ClickThroughMode::Passthrough);
        assert!(state.should_ignore(5.0, 5.0));
    }
}
