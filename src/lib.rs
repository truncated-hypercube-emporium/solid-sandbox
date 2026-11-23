//! Solid Sandbox for Android 4.4+ (NativeActivity, no Java code) and iOS 14+
//! (a static library driven by the UIKit shell in `ios/`).
//!
//! A random number of platomon's path-traced solids bounce around a cube
//! about 10 solid radii on a side. Each solid has its own colour gradient and
//! sits on it by speed. Tap a solid to send it off in a random direction at a
//! fixed speed. The button in the top-right corner opens the settings
//! dialog; Back closes it, or exits. No system monitoring: platomon is built without
//! its `monitor` feature, so no collector, metrics or stats bar.

#![allow(non_camel_case_types, non_snake_case, clippy::missing_safety_doc)]
// The pixel helpers are only called from the Android glue (and tests).
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

#[cfg(target_os = "android")]
#[path = "../android/src/compat.rs"]
mod compat;
#[path = "../android/src/font.rs"]
mod font;
pub mod app;
#[cfg(any(target_os = "ios", test))]
pub mod ios;
pub mod palette;
pub mod physics;
pub mod sandbox;
pub mod settings;
pub mod shade;
pub mod ui;

use font::{CELL_H, CELL_W, Font, GLYPH_W};

/// Share of wall time spent rendering at the frame-rate cap (all cores).
const BUDGET: f32 = 0.9;
/// Phones of the Android 4 era: cap at 30 fps for battery and heat.
const FPS_CAP: f32 = 30.0;
/// Minimum cells on screen, even if a slow device drops below the fps cap.
const MIN_CELLS: f32 = 3000.0;
/// Character density multiplier: 4× the cells the budget alone would give
/// (half the cell width and height).
const DENSITY: f32 = 4.0;
/// Largest pixel scale of a font cell (4 → 24×32 px cells).
const MAX_SCALE: i32 = 4;

/// Largest pixel scale with the "Character scale" setting at 2.
const MAX_CHAR_SCALE: i32 = 2 * MAX_SCALE;

/// The cell scale for the "Character scale" setting: `base` (from
/// [`scale_for`]) times `factor`, rounded, at least 1 pixel per font pixel.
fn char_scale(base: i32, factor: f32) -> i32 {
    ((base as f32 * factor).round() as i32).clamp(1, MAX_CHAR_SCALE)
}

/// Pixel scale for the 6×8 font cell so a full screen of cells stays within
/// [`BUDGET`] at [`FPS_CAP`], but never coarser than [`MIN_CELLS`] / [`MAX_SCALE`].
fn scale_for(w: i32, h: i32, sec_per_cell: f32) -> i32 {
    let cells_max = DENSITY * (BUDGET / (FPS_CAP * sec_per_cell.max(1e-9))).max(MIN_CELLS);
    let per_cell_px = (w.max(1) * h.max(1)) as f32 / cells_max;
    ((per_cell_px / (CELL_W * CELL_H) as f32).sqrt().ceil() as i32).clamp(1, MAX_SCALE)
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct Grid {
    w: i32,
    h: i32,
    scale: i32,
    cols: usize,
    rows: usize,
    x0: i32,
    y0: i32,
}

impl Grid {
    fn new(w: i32, h: i32, scale: i32) -> Self {
        let (cw, ch) = (CELL_W as i32 * scale, CELL_H as i32 * scale);
        let cols = (w / cw).max(1) as usize;
        let rows = (h / ch).max(1) as usize;
        let x0 = (w - cols as i32 * cw) / 2;
        let y0 = (h - rows as i32 * ch) / 2;
        Self { w, h, scale, cols, rows, x0, y0 }
    }

    /// Screen pixel → continuous grid position (columns, rows).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    fn to_grid(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x0 as f32) / (CELL_W as i32 * self.scale) as f32, (y - self.y0 as f32) / (CELL_H as i32 * self.scale) as f32)
    }
}

/// RGBX_8888 pixel (red in the low byte) from `0xRRGGBB`.
#[inline]
fn rgbx(c: u32) -> u32 {
    0xFF00_0000 | ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff)
}

/// Draw one cell row of coloured text into the locked buffer.
fn draw_row(px: &mut [u32], stride: usize, g: &Grid, font: &Font, row: usize, cells: &[char], colors: &[u32]) {
    let s = g.scale as usize;
    let y_top = g.y0 as usize + row * CELL_H * s;
    for (i, &c) in cells.iter().enumerate() {
        if c == ' ' {
            continue;
        }
        let ink = rgbx(colors.get(i).copied().unwrap_or(0xcccccc));
        let x_left = g.x0 as usize + i * CELL_W * s;
        for (gy, bits) in font.get(c).iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            for gx in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - gx)) == 0 {
                    continue;
                }
                for dy in 0..s {
                    let y = y_top + gy * s + dy;
                    let start = y * stride + x_left + gx * s;
                    if let Some(span) = px.get_mut(start..start + s) {
                        span.fill(ink);
                    }
                }
            }
        }
    }
}

/// The NativeActivity glue and render loop (Android only; the helpers above
/// and the sandbox are plain Rust and tested on the host).
#[cfg(target_os = "android")]
pub mod native {
    use std::ffi::{CString, c_char, c_void};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use crate::app::App;

    #[repr(C)]
    pub struct ANativeActivityCallbacks {
        onStart: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onResume: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onSaveInstanceState: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut usize) -> *mut c_void>,
        onPause: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onStop: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onDestroy: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onWindowFocusChanged: Option<unsafe extern "C" fn(*mut ANativeActivity, i32)>,
        onNativeWindowCreated: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onNativeWindowResized: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onNativeWindowRedrawNeeded: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onNativeWindowDestroyed: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onInputQueueCreated: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onInputQueueDestroyed: Option<unsafe extern "C" fn(*mut ANativeActivity, *mut c_void)>,
        onContentRectChanged: Option<unsafe extern "C" fn(*mut ANativeActivity, *const c_void)>,
        onConfigurationChanged: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
        onLowMemory: Option<unsafe extern "C" fn(*mut ANativeActivity)>,
    }

    #[repr(C)]
    pub struct ANativeActivity {
        callbacks: *mut ANativeActivityCallbacks,
        vm: *mut c_void,
        env: *mut c_void,
        clazz: *mut c_void,
        internalDataPath: *const c_char,
        externalDataPath: *const c_char,
        sdkVersion: i32,
        instance: *mut c_void,
        assetManager: *mut c_void,
        obbPath: *const c_char,
    }

    #[repr(C)]
    struct ANativeWindow_Buffer {
        width: i32,
        height: i32,
        stride: i32,
        format: i32,
        bits: *mut c_void,
        reserved: [u32; 6],
    }

    #[link(name = "android")]
    unsafe extern "C" {
        fn ANativeWindow_setBuffersGeometry(w: *mut c_void, width: i32, height: i32, format: i32) -> i32;
        fn ANativeWindow_getWidth(w: *mut c_void) -> i32;
        fn ANativeWindow_getHeight(w: *mut c_void) -> i32;
        fn ANativeWindow_lock(w: *mut c_void, out: *mut ANativeWindow_Buffer, dirty: *mut c_void) -> i32;
        fn ANativeWindow_unlockAndPost(w: *mut c_void) -> i32;
        fn ALooper_prepare(opts: i32) -> *mut c_void;
        fn ALooper_pollAll(timeout: i32, fd: *mut i32, events: *mut i32, data: *mut *mut c_void) -> i32;
        fn AInputQueue_attachLooper(
            q: *mut c_void,
            looper: *mut c_void,
            ident: i32,
            cb: Option<unsafe extern "C" fn(i32, i32, *mut c_void) -> i32>,
            data: *mut c_void,
        );
        fn AInputQueue_detachLooper(q: *mut c_void);
        fn AInputQueue_getEvent(q: *mut c_void, out: *mut *mut c_void) -> i32;
        fn AInputQueue_preDispatchEvent(q: *mut c_void, e: *mut c_void) -> i32;
        fn AInputQueue_finishEvent(q: *mut c_void, e: *mut c_void, handled: i32);
        fn AInputEvent_getType(e: *const c_void) -> i32;
        fn AKeyEvent_getAction(e: *const c_void) -> i32;
        fn AKeyEvent_getKeyCode(e: *const c_void) -> i32;
        fn AMotionEvent_getAction(e: *const c_void) -> i32;
        fn AMotionEvent_getX(e: *const c_void, pointer: usize) -> f32;
        fn AMotionEvent_getY(e: *const c_void, pointer: usize) -> f32;
        fn ANativeActivity_finish(a: *mut ANativeActivity);
        fn ANativeActivity_setWindowFlags(a: *mut ANativeActivity, add: u32, remove: u32);
        fn ASensorManager_getInstance() -> *mut c_void;
        fn ASensorManager_getDefaultSensor(m: *mut c_void, kind: i32) -> *const c_void;
        fn ASensorManager_createEventQueue(
            m: *mut c_void,
            looper: *mut c_void,
            ident: i32,
            cb: Option<unsafe extern "C" fn(i32, i32, *mut c_void) -> i32>,
            data: *mut c_void,
        ) -> *mut c_void;
        fn ASensorManager_destroyEventQueue(m: *mut c_void, q: *mut c_void) -> i32;
        fn ASensorEventQueue_enableSensor(q: *mut c_void, s: *const c_void) -> i32;
        fn ASensorEventQueue_disableSensor(q: *mut c_void, s: *const c_void) -> i32;
        fn ASensorEventQueue_setEventRate(q: *mut c_void, s: *const c_void, usec: i32) -> i32;
        fn ASensorEventQueue_getEvents(q: *mut c_void, events: *mut ASensorEvent, count: usize) -> isize;
    }

    /// `ASensorEvent` from `<android/sensor.h>` (104 bytes on 32-bit ARM).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ASensorEvent {
        version: i32,
        sensor: i32,
        kind: i32,
        reserved0: i32,
        timestamp: i64,
        data: [f32; 16],
        flags: u32,
        reserved1: [i32; 3],
    }

    const ASENSOR_TYPE_ACCELEROMETER: i32 = 1;
    const LOOPER_ID_SENSOR: i32 = 2;
    /// Accelerometer sampling period: 50 Hz.
    const SENSOR_PERIOD_US: i32 = 20_000;

    /// The raw accelerometer, read through an event queue on the render
    /// thread's looper; enabled only while gravity is on and the app is in
    /// front, to save battery.
    struct Accel {
        mgr: *mut c_void,
        sensor: *const c_void,
        queue: *mut c_void,
        enabled: bool,
        /// Latest raw reading, m/s² in device axes.
        latest: Option<[f32; 3]>,
        /// Last reading written to the log.
        logged: Option<[f32; 3]>,
    }

    impl Accel {
        /// # Safety
        /// `looper` is the calling thread's prepared looper.
        unsafe fn new(looper: *mut c_void) -> Self {
            // SAFETY: plain NDK calls; null results mean no sensor.
            unsafe {
                let mgr = ASensorManager_getInstance();
                let sensor =
                    if mgr.is_null() { std::ptr::null() } else { ASensorManager_getDefaultSensor(mgr, ASENSOR_TYPE_ACCELEROMETER) };
                let queue = if sensor.is_null() {
                    std::ptr::null_mut()
                } else {
                    ASensorManager_createEventQueue(mgr, looper, LOOPER_ID_SENSOR, None, std::ptr::null_mut())
                };
                log(if queue.is_null() { "no accelerometer" } else { "accelerometer found" });
                Self { mgr, sensor, queue, enabled: false, latest: None, logged: None }
            }
        }

        fn set_enabled(&mut self, on: bool) {
            if on == self.enabled || self.queue.is_null() {
                return;
            }
            // SAFETY: our queue and the sensor it was created for.
            unsafe {
                if on {
                    ASensorEventQueue_enableSensor(self.queue, self.sensor);
                    ASensorEventQueue_setEventRate(self.queue, self.sensor, SENSOR_PERIOD_US);
                } else {
                    ASensorEventQueue_disableSensor(self.queue, self.sensor);
                    self.latest = None;
                    self.logged = None;
                }
            }
            self.enabled = on;
        }

        /// Read every pending event, keeping the newest reading.
        fn drain(&mut self) {
            if self.queue.is_null() {
                return;
            }
            // SAFETY: an all-zero event is a valid value; getEvents writes at
            // most `len` events into the array.
            let mut ev: [ASensorEvent; 8] = unsafe { std::mem::zeroed() };
            loop {
                let n = unsafe { ASensorEventQueue_getEvents(self.queue, ev.as_mut_ptr(), ev.len()) };
                if n <= 0 {
                    break;
                }
                for e in &ev[..n as usize] {
                    if e.kind == ASENSOR_TYPE_ACCELEROMETER {
                        let a = [e.data[0], e.data[1], e.data[2]];
                        // log the first reading and every big change (a tilt, not jitter)
                        let moved = self.logged.is_none_or(|l| (0..3).map(|k| (a[k] - l[k]).powi(2)).sum::<f32>() > 9.0);
                        if moved {
                            log(&format!("accelerometer {:.2} {:.2} {:.2}", a[0], a[1], a[2]));
                            self.logged = Some(a);
                        }
                        self.latest = Some(a);
                    }
                }
            }
        }
    }

    impl Drop for Accel {
        fn drop(&mut self) {
            self.set_enabled(false);
            if !self.queue.is_null() {
                // SAFETY: the queue we created from this manager.
                unsafe { ASensorManager_destroyEventQueue(self.mgr, self.queue) };
            }
        }
    }

    /// The display rotation (`Surface.ROTATION_0..270` as 0..3), which only
    /// Java knows: the landscape screen may be either way up. Calls
    /// `activity.getWindowManager().getDefaultDisplay().getRotation()`
    /// through JNI from the render thread, attached for its lifetime.
    struct Rotation {
        vm: *mut *const *const c_void,
        env: *mut *const *const c_void,
        activity: *mut c_void,
    }

    /// JNI function-table indices (`<jni.h>`).
    const JNI_DELETE_LOCAL_REF: usize = 23;
    const JNI_EXCEPTION_CLEAR: usize = 17;
    const JNI_GET_OBJECT_CLASS: usize = 31;
    const JNI_GET_METHOD_ID: usize = 33;
    const JNI_CALL_OBJECT_METHOD_A: usize = 36;
    const JNI_CALL_INT_METHOD_A: usize = 51;
    const JNI_EXCEPTION_CHECK: usize = 228;
    const JVM_ATTACH_CURRENT_THREAD: usize = 4;
    const JVM_DETACH_CURRENT_THREAD: usize = 5;

    impl Rotation {
        /// # Safety
        /// `a` is the live activity, which outlives this thread.
        unsafe fn attach(a: *const ANativeActivity) -> Option<Self> {
            // SAFETY: the activity's JavaVM; AttachCurrentThread fills `env`.
            unsafe {
                let vm = (*a).vm as *mut *const *const c_void;
                if vm.is_null() {
                    return None;
                }
                let attach: unsafe extern "C" fn(*mut c_void, *mut *mut c_void, *mut c_void) -> i32 =
                    std::mem::transmute(*(*vm).add(JVM_ATTACH_CURRENT_THREAD));
                let mut env: *mut c_void = std::ptr::null_mut();
                if attach(vm as *mut c_void, &mut env, std::ptr::null_mut()) != 0 || env.is_null() {
                    log("JNI attach failed");
                    return None;
                }
                Some(Self { vm, env: env as *mut *const *const c_void, activity: (*a).clazz })
            }
        }

        /// # Safety
        /// `T` is the exact type of JNI function `i`.
        unsafe fn f<T: Copy>(&self, i: usize) -> T {
            // SAFETY: reads entry `i` of the env's function table.
            unsafe { std::mem::transmute_copy(&*(*self.env).add(i)) }
        }

        /// `obj.name()` returning an object (`ret` is the JNI signature).
        unsafe fn call_object(&self, obj: *mut c_void, name: &std::ffi::CStr, sig: &std::ffi::CStr) -> *mut c_void {
            // SAFETY: JNI calls with this thread's env on a live local/global ref.
            unsafe {
                let env = self.env as *mut c_void;
                let get_class: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = self.f(JNI_GET_OBJECT_CLASS);
                let get_mid: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char, *const c_char) -> *mut c_void =
                    self.f(JNI_GET_METHOD_ID);
                let call: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_void) -> *mut c_void =
                    self.f(JNI_CALL_OBJECT_METHOD_A);
                let cls = get_class(env, obj);
                if cls.is_null() || self.failed() {
                    return std::ptr::null_mut();
                }
                let mid = get_mid(env, cls, name.as_ptr(), sig.as_ptr());
                self.delete(cls);
                if mid.is_null() || self.failed() {
                    return std::ptr::null_mut();
                }
                let out = call(env, obj, mid, std::ptr::null());
                if self.failed() { std::ptr::null_mut() } else { out }
            }
        }

        /// A pending Java exception: clear it and report failure.
        unsafe fn failed(&self) -> bool {
            // SAFETY: JNI calls with this thread's env.
            unsafe {
                let check: unsafe extern "C" fn(*mut c_void) -> u8 = self.f(JNI_EXCEPTION_CHECK);
                let clear: unsafe extern "C" fn(*mut c_void) = self.f(JNI_EXCEPTION_CLEAR);
                let env = self.env as *mut c_void;
                if check(env) != 0 {
                    clear(env);
                    true
                } else {
                    false
                }
            }
        }

        unsafe fn delete(&self, obj: *mut c_void) {
            // SAFETY: a local ref this thread created.
            unsafe {
                if !obj.is_null() {
                    let del: unsafe extern "C" fn(*mut c_void, *mut c_void) = self.f(JNI_DELETE_LOCAL_REF);
                    del(self.env as *mut c_void, obj);
                }
            }
        }

        fn get(&self) -> Option<i32> {
            // SAFETY: the activity is a global ref; every local ref made here
            // is deleted (this thread never returns to Java to free them).
            unsafe {
                let wm = self.call_object(self.activity, c"getWindowManager", c"()Landroid/view/WindowManager;");
                if wm.is_null() {
                    return None;
                }
                let display = self.call_object(wm, c"getDefaultDisplay", c"()Landroid/view/Display;");
                self.delete(wm);
                if display.is_null() {
                    return None;
                }
                let env = self.env as *mut c_void;
                let get_class: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = self.f(JNI_GET_OBJECT_CLASS);
                let get_mid: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_char, *const c_char) -> *mut c_void =
                    self.f(JNI_GET_METHOD_ID);
                let call_int: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_void) -> i32 =
                    self.f(JNI_CALL_INT_METHOD_A);
                let cls = get_class(env, display);
                let mid = if cls.is_null() { std::ptr::null_mut() } else { get_mid(env, cls, c"getRotation".as_ptr(), c"()I".as_ptr()) };
                self.delete(cls);
                let rot = if mid.is_null() || self.failed() { None } else { Some(call_int(env, display, mid, std::ptr::null())) };
                self.delete(display);
                if self.failed() { None } else { rot }
            }
        }
    }

    impl Drop for Rotation {
        fn drop(&mut self) {
            // SAFETY: detaches the thread attached in `attach`.
            unsafe {
                let detach: unsafe extern "C" fn(*mut c_void) -> i32 = std::mem::transmute(*(*self.vm).add(JVM_DETACH_CURRENT_THREAD));
                detach(self.vm as *mut c_void);
            }
        }
    }

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
    }

    const WINDOW_FORMAT_RGBX_8888: i32 = 2;
    const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: i32 = 1;
    const LOOPER_ID_INPUT: i32 = 1;
    const AINPUT_EVENT_TYPE_KEY: i32 = 1;
    const AINPUT_EVENT_TYPE_MOTION: i32 = 2;
    const AKEY_EVENT_ACTION_UP: i32 = 1;
    const AKEYCODE_BACK: i32 = 4;
    const AMOTION_EVENT_ACTION_DOWN: i32 = 0;
    const AMOTION_EVENT_ACTION_POINTER_DOWN: i32 = 5;
    const AWINDOW_FLAG_KEEP_SCREEN_ON: u32 = 0x80;

    fn log(msg: &str) {
        let tag = c"solidsandbox";
        if let Ok(text) = CString::new(msg.replace('\0', " ")) {
            // SAFETY: NUL-terminated strings for the duration of the call.
            unsafe { __android_log_write(4, tag.as_ptr(), text.as_ptr()) };
        }
    }

    /// What the activity callbacks tell the render thread. Raw handles are
    /// stored as `usize`; the window is only touched while `state` is
    /// locked, and `onNativeWindowDestroyed` takes the lock before
    /// returning, so the window is never used after Android takes it back.
    #[derive(Default)]
    struct State {
        window: usize,
        window_changed: bool,
        queue: usize,
        looper: usize,
        resumed: bool,
        quit: bool,
    }

    struct Shared {
        state: Mutex<State>,
        wake: Condvar,
        activity: usize,
        thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    unsafe fn shared(a: *mut ANativeActivity) -> Option<&'static Shared> {
        // SAFETY: `instance` holds an `Arc<Shared>` from
        // `ANativeActivity_onCreate` until `on_destroy` clears it.
        unsafe {
            let p = (*a).instance as *const Shared;
            p.as_ref()
        }
    }

    fn with_state(a: *mut ANativeActivity, f: impl FnOnce(&mut State)) {
        // SAFETY: called from activity callbacks with the activity Android passed.
        if let Some(s) = unsafe { shared(a) } {
            let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut st);
            s.wake.notify_all();
        }
    }

    unsafe extern "C" fn on_resume(a: *mut ANativeActivity) {
        with_state(a, |s| s.resumed = true);
    }

    unsafe extern "C" fn on_pause(a: *mut ANativeActivity) {
        with_state(a, |s| s.resumed = false);
    }

    unsafe extern "C" fn on_window_created(a: *mut ANativeActivity, w: *mut c_void) {
        with_state(a, |s| {
            s.window = w as usize;
            s.window_changed = true;
        });
    }

    unsafe extern "C" fn on_window_resized(a: *mut ANativeActivity, _w: *mut c_void) {
        with_state(a, |s| s.window_changed = true);
    }

    unsafe extern "C" fn on_window_destroyed(a: *mut ANativeActivity, _w: *mut c_void) {
        with_state(a, |s| s.window = 0);
    }

    unsafe extern "C" fn on_input_created(a: *mut ANativeActivity, q: *mut c_void) {
        with_state(a, |s| {
            s.queue = q as usize;
            if s.looper != 0 {
                // SAFETY: a live queue and the render thread's looper.
                unsafe { AInputQueue_attachLooper(q, s.looper as *mut c_void, LOOPER_ID_INPUT, None, std::ptr::null_mut()) };
            }
        });
    }

    unsafe extern "C" fn on_input_destroyed(a: *mut ANativeActivity, q: *mut c_void) {
        with_state(a, |s| {
            if s.queue != 0 && s.looper != 0 {
                // SAFETY: the queue we attached.
                unsafe { AInputQueue_detachLooper(q) };
            }
            s.queue = 0;
        });
    }

    unsafe extern "C" fn on_destroy(a: *mut ANativeActivity) {
        // SAFETY: the Arc stored in `instance` by `ANativeActivity_onCreate`.
        unsafe {
            let p = (*a).instance as *const Shared;
            if p.is_null() {
                return;
            }
            with_state(a, |s| s.quit = true);
            let handle = (*p).thread.lock().ok().and_then(|mut t| t.take());
            if let Some(h) = handle {
                let _ = h.join();
            }
            (*a).instance = std::ptr::null_mut();
            drop(Arc::from_raw(p));
        }
    }

    /// Entry point called by `android.app.NativeActivity`.
    ///
    /// # Safety
    /// Called by Android with a valid activity.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn ANativeActivity_onCreate(activity: *mut ANativeActivity, _saved: *mut c_void, _size: usize) {
        std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
        // SAFETY: Android hands us a valid activity whose callbacks we may set.
        unsafe {
            let cb = &mut *(*activity).callbacks;
            cb.onResume = Some(on_resume);
            cb.onPause = Some(on_pause);
            cb.onDestroy = Some(on_destroy);
            cb.onNativeWindowCreated = Some(on_window_created);
            cb.onNativeWindowResized = Some(on_window_resized);
            cb.onNativeWindowDestroyed = Some(on_window_destroyed);
            cb.onInputQueueCreated = Some(on_input_created);
            cb.onInputQueueDestroyed = Some(on_input_destroyed);
            ANativeActivity_setWindowFlags(activity, AWINDOW_FLAG_KEEP_SCREEN_ON, 0);

            let shared = Arc::new(Shared {
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
                activity: activity as usize,
                thread: Mutex::new(None),
            });
            let worker = Arc::clone(&shared);
            let handle = std::thread::Builder::new()
                .name("sandbox-render".into())
                .spawn(move || {
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render_thread(&worker))).is_err() {
                        log("render thread panicked; closing");
                        ANativeActivity_finish(worker.activity as *mut ANativeActivity);
                    }
                })
                .ok();
            *shared.thread.lock().unwrap_or_else(|e| e.into_inner()) = handle;
            (*activity).instance = Arc::into_raw(shared) as *mut c_void;
        }
        log("created");
    }

    /// Drain input: each finger going down is a tap (screen px); Back
    /// releases are counted in `backs`.
    fn poll_input(sh: &Shared, taps: &mut Vec<(f32, f32)>, backs: &mut u32, accel: &mut Accel) {
        // SAFETY: looper/queue handles are only used while the state lock is
        // held, and the callbacks that change them take the same lock.
        unsafe {
            let mut events = 0;
            let mut data = std::ptr::null_mut();
            loop {
                match ALooper_pollAll(0, std::ptr::null_mut(), &mut events, &mut data) {
                    LOOPER_ID_INPUT => {}
                    LOOPER_ID_SENSOR => {
                        accel.drain();
                        continue;
                    }
                    _ => break,
                }
                let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
                if st.queue == 0 {
                    break;
                }
                let q = st.queue as *mut c_void;
                let mut ev = std::ptr::null_mut();
                while AInputQueue_getEvent(q, &mut ev) >= 0 {
                    if AInputQueue_preDispatchEvent(q, ev) != 0 {
                        continue;
                    }
                    let mut handled = 0;
                    match AInputEvent_getType(ev) {
                        AINPUT_EVENT_TYPE_KEY if AKeyEvent_getKeyCode(ev) == AKEYCODE_BACK => {
                            if AKeyEvent_getAction(ev) == AKEY_EVENT_ACTION_UP {
                                *backs += 1;
                            }
                            handled = 1;
                        }
                        AINPUT_EVENT_TYPE_MOTION => {
                            let action = AMotionEvent_getAction(ev);
                            let pointer = ((action >> 8) & 0xff) as usize;
                            match action & 0xff {
                                AMOTION_EVENT_ACTION_DOWN => taps.push((AMotionEvent_getX(ev, 0), AMotionEvent_getY(ev, 0))),
                                AMOTION_EVENT_ACTION_POINTER_DOWN => {
                                    taps.push((AMotionEvent_getX(ev, pointer), AMotionEvent_getY(ev, pointer)))
                                }
                                _ => {}
                            }
                            handled = 1;
                        }
                        _ => {}
                    }
                    AInputQueue_finishEvent(q, ev, handled);
                }
            }
        }
    }

    fn render_thread(sh: &Shared) {
        // SAFETY: the activity outlives this thread (on_destroy joins it).
        let rotation = unsafe { Rotation::attach(sh.activity as *const ANativeActivity) };
        let mut accel = {
            // SAFETY: prepares a looper for the calling thread.
            let looper = unsafe { ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) } as usize;
            let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            st.looper = looper;
            if st.queue != 0 {
                // SAFETY: a live queue and our looper.
                unsafe {
                    AInputQueue_attachLooper(
                        st.queue as *mut c_void,
                        looper as *mut c_void,
                        LOOPER_ID_INPUT,
                        None,
                        std::ptr::null_mut(),
                    )
                };
            }
            drop(st);
            // SAFETY: our thread's prepared looper.
            unsafe { Accel::new(looper as *mut c_void) }
        };

        let (w, h) = {
            let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            while st.window == 0 && !st.quit {
                st = sh.wake.wait_timeout(st, Duration::from_millis(100)).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
            }
            if st.quit {
                return;
            }
            let win = st.window as *mut c_void;
            // SAFETY: the window is live while we hold the lock.
            unsafe { (ANativeWindow_getWidth(win), ANativeWindow_getHeight(win)) }
        };

        // SAFETY: the activity outlives this thread (on_destroy joins it);
        // internalDataPath is a NUL-terminated string Android owns.
        let data_dir = unsafe {
            let p = (*(sh.activity as *const ANativeActivity)).internalDataPath;
            (!p.is_null()).then(|| std::path::PathBuf::from(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()))
        };
        let mut app = App::new(w, h, data_dir, log);
        let mut taps: Vec<(f32, f32)> = Vec::new();
        let mut backs = 0u32;
        let mut last = Instant::now();
        // display rotation: landscape is 1 (90°) unless Java says otherwise;
        // re-read twice a second while gravity is on, as a 180° flip of a
        // landscape phone changes neither the window size nor the config
        let mut rot = 1;
        let mut rot_checked: Option<Instant> = None;

        loop {
            // taps accumulate across the idle passes until the next frame applies them
            poll_input(sh, &mut taps, &mut backs, &mut accel);
            for _ in 0..std::mem::take(&mut backs) {
                if !app.back() {
                    // SAFETY: the activity is live while this thread runs.
                    unsafe { ANativeActivity_finish(sh.activity as *mut ANativeActivity) };
                }
            }
            {
                let st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
                if st.quit {
                    break;
                }
                if !st.resumed || st.window == 0 {
                    taps.clear(); // nothing on screen to tap
                    accel.set_enabled(false);
                    drop(sh.wake.wait_timeout(st, Duration::from_millis(50)));
                    last = Instant::now();
                    continue;
                }
            }
            // the frame-rate cap
            let period = Duration::from_secs_f32(1.0 / app.fps());
            let now = Instant::now();
            if now < last + period {
                std::thread::sleep((last + period - now).min(Duration::from_millis(8)));
                continue;
            }
            let dt = (now - last).as_secs_f32();
            last = now;
            for (x, y) in taps.drain(..) {
                app.tap(x, y);
            }
            let gravity_on = app.wants_accel();
            accel.set_enabled(gravity_on);
            if gravity_on {
                accel.drain();
                if rot_checked.is_none_or(|t| now - t >= Duration::from_millis(500)) {
                    rot_checked = Some(now);
                    if let Some(r) = rotation.as_ref().and_then(Rotation::get)
                        && r != rot
                    {
                        log(&format!("display rotation {r}"));
                        rot = r;
                    }
                }
            }
            app.step(dt, accel.latest.map(|a| (a, rot)));

            let mut st = sh.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.window == 0 {
                continue;
            }
            let win = st.window as *mut c_void;
            // SAFETY: the window is live while we hold the lock; the buffer is
            // only written inside its locked bounds.
            unsafe {
                if st.window_changed {
                    st.window_changed = false;
                    ANativeWindow_setBuffersGeometry(win, 0, 0, WINDOW_FORMAT_RGBX_8888);
                    let (nw, nh) = (ANativeWindow_getWidth(win), ANativeWindow_getHeight(win));
                    if (nw, nh) != app.size() {
                        // the first window can be a placeholder size: re-fit the cells
                        app.resize(nw, nh);
                        continue; // next frame renders at the new grid size
                    }
                }
                let mut buf = ANativeWindow_Buffer {
                    width: 0,
                    height: 0,
                    stride: 0,
                    format: 0,
                    bits: std::ptr::null_mut(),
                    reserved: [0; 6],
                };
                if ANativeWindow_lock(win, &mut buf, std::ptr::null_mut()) != 0 || buf.bits.is_null() {
                    continue;
                }
                let stride = buf.stride.max(buf.width) as usize;
                let px = std::slice::from_raw_parts_mut(buf.bits as *mut u32, stride * buf.height.max(0) as usize);
                app.draw(px, stride, buf.width, buf.height);
                ANativeWindow_unlockAndPost(win);
            }
        }
        log("render thread done");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_keeps_cells_within_dense_budget() {
        let s = scale_for(1280, 720, 4e-6);
        let g = Grid::new(1280, 720, s);
        let load = (g.cols * g.rows) as f32 * 4e-6 * FPS_CAP;
        assert!(load <= DENSITY * BUDGET * 1.05, "scale {s}: {}x{} cells, load {load}", g.cols, g.rows);
        assert_eq!(scale_for(1280, 720, 1e-9), 1);
        // the emulator measured 7.55 us/cell at 2009x1080: scale 4 before, 2 now
        assert_eq!(scale_for(2009, 1080, 7.55e-6), 2);
        assert!(scale_for(2009, 1080, 1.0) <= MAX_SCALE);
    }

    #[test]
    fn character_scale_halves_and_doubles_the_cells() {
        assert_eq!((char_scale(2, 0.5), char_scale(2, 1.0), char_scale(2, 2.0)), (1, 2, 4));
        assert_eq!(char_scale(4, 2.0), 8);
        assert_eq!(char_scale(1, 0.5), 1, "never below one pixel per font pixel");
        let (half, full) = (Grid::new(2009, 1080, char_scale(2, 0.5)), Grid::new(2009, 1080, 2));
        assert!(half.cols >= full.cols * 2 - 1 && half.rows >= full.rows * 2 - 1);
    }

    #[test]
    fn taps_map_to_grid_cells() {
        let g = Grid::new(1280, 720, 3);
        let (cx, cy) = g.to_grid(g.x0 as f32 + 18.0 * 5.5, g.y0 as f32 + 24.0 * 2.25);
        assert!((cx - 5.5).abs() < 1e-4 && (cy - 2.25).abs() < 1e-4);
    }

    #[test]
    fn rows_draw_in_colour_inside_the_buffer() {
        let g = Grid::new(64, 40, 2);
        let stride = 70;
        let mut px = vec![0u32; stride * 40];
        let font = Font::default();
        let row: Vec<char> = "#@ab".chars().chain(std::iter::repeat(' ')).take(g.cols).collect();
        let colors = vec![0x112233; g.cols];
        draw_row(&mut px, stride, &g, &font, 0, &row, &colors);
        assert!(px.iter().any(|&p| p == 0xFF33_2211));
        for y in 0..40 {
            assert!(px[y * stride + 64..y * stride + 70].iter().all(|&p| p == 0));
        }
    }
}
