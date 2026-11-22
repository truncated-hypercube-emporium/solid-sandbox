//! The C interface the iOS shell (`ios/SolidSandbox`) drives the app
//! through. Declared for Objective-C in `ios/include/solid_sandbox.h`; every
//! call happens on the main thread.

use std::ffi::{CStr, c_char};

use crate::app::App;

/// Standard gravity, m/s²: CoreMotion reports in g.
const G: f32 = 9.806_65;

fn log(msg: &str) {
    eprintln!("solidsandbox: {msg}");
}

/// iOS accelerometer reading (g, device axes: x right, y to the top edge, z
/// out of the screen; it reads (0, -1, 0) standing upright) as Android's raw
/// reading: the same axes, in m/s², the other sign (Android reports the push
/// that holds the device up, (0, +g, 0) upright).
pub fn android_accel(ios: [f32; 3]) -> [f32; 3] {
    ios.map(|a| -a * G)
}

/// Make the app for a `w × h` pixel view, saving its settings in `data_dir`
/// (a NUL-terminated UTF-8 path, or null for none). Free with [`ss_destroy`].
///
/// # Safety
/// `data_dir` is null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ss_create(w: i32, h: i32, data_dir: *const c_char) -> *mut App {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
    // SAFETY: the caller passes null or a valid C string.
    let dir = (!data_dir.is_null()).then(|| std::path::PathBuf::from(unsafe { CStr::from_ptr(data_dir) }.to_string_lossy().into_owned()));
    match std::panic::catch_unwind(|| App::new(w.max(1), h.max(1), dir, log)) {
        Ok(app) => Box::into_raw(Box::new(app)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `app` came from [`ss_create`] and is not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ss_destroy(app: *mut App) {
    if !app.is_null() {
        // SAFETY: the Box made in ss_create.
        drop(unsafe { Box::from_raw(app) });
    }
}

/// Run `f` on the app unless it is null; a panic is logged and swallowed so
/// it never unwinds into Objective-C.
fn with<R: Default>(app: *mut App, f: impl FnOnce(&mut App) -> R) -> R {
    // SAFETY: the shell only passes pointers from ss_create, on one thread.
    match unsafe { app.as_mut() } {
        Some(a) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(a))).unwrap_or_default(),
        None => R::default(),
    }
}

/// The view's new pixel size.
#[unsafe(no_mangle)]
pub extern "C" fn ss_resize(app: *mut App, w: i32, h: i32) {
    with(app, |a| a.resize(w.max(1), h.max(1)));
}

/// A finger going down at view pixel (`x`, `y`).
#[unsafe(no_mangle)]
pub extern "C" fn ss_tap(app: *mut App, x: f32, y: f32) {
    with(app, |a| a.tap(x, y));
}

/// The frame-rate cap, frames per second.
#[unsafe(no_mangle)]
pub extern "C" fn ss_fps(app: *mut App) -> f32 {
    with(app, |a| a.fps())
}

/// Whether gravity is on, so the shell should run the accelerometer.
#[unsafe(no_mangle)]
pub extern "C" fn ss_wants_accel(app: *mut App) -> bool {
    with(app, |a| a.wants_accel())
}

/// Advance `dt` real seconds and render. With `has_accel`, (`ax`, `ay`,
/// `az`) is CoreMotion's accelerometer reading in g and `rotation` the
/// interface orientation as Android's `Surface.ROTATION_*` (0 portrait, 1
/// landscape with the top edge on the left (UIInterfaceOrientationLandscapeRight),
/// 2 upside down, 3 landscape with the top edge on the right).
#[unsafe(no_mangle)]
pub extern "C" fn ss_step(app: *mut App, dt: f32, has_accel: bool, ax: f32, ay: f32, az: f32, rotation: i32) {
    let reading = has_accel.then(|| (android_accel([ax, ay, az]), rotation));
    with(app, |a| a.step(dt, reading));
}

/// Draw the frame into `pixels`: `h` rows of `stride` 32-bit pixels, each
/// bytes R, G, B, X in memory (`kCGImageAlphaNoneSkipLast |
/// kCGBitmapByteOrder32Big`), `w` of them used per row.
///
/// # Safety
/// `pixels` points to `stride × h` writable `u32`s, `stride >= w`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ss_draw(app: *mut App, pixels: *mut u32, stride: i32, w: i32, h: i32) {
    if pixels.is_null() || w <= 0 || h <= 0 || stride < w {
        return;
    }
    // SAFETY: the caller's buffer, as documented.
    let px = unsafe { std::slice::from_raw_parts_mut(pixels, stride as usize * h as usize) };
    with(app, |a| a.draw(px, stride as usize, w, h));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_readings_become_android_ones() {
        // standing upright: iOS (0, -1, 0) g, Android (0, +g, 0)
        assert_eq!(android_accel([0.0, -1.0, 0.0]), [0.0, G, 0.0]);
    }
}
