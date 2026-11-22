//! Android 4.4 (API 19) compatibility shims.
//!
//! Rust's standard library and some dependencies reference a few libc
//! functions that bionic only gained in Android 5.0+ (`readlinkat`,
//! `dl_iterate_phdr` on ARM) or 7.0 (`getifaddrs`). A strong reference to a
//! missing symbol stops the library from loading at all, so these
//! definitions satisfy the references inside this `.so`: each forwards to the
//! system's own function when the device has it (looked up in libc at run
//! time) and otherwise falls back or reports `ENOSYS`.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

unsafe extern "C" {
    fn __errno() -> *mut c_int;
    fn readlink(path: *const c_char, buf: *mut c_char, size: usize) -> isize;
}

const RTLD_NOW: c_int = 2;
const ENOSYS: c_int = 38;
const AT_FDCWD: c_int = -100;
/// "Not looked up yet" (0 means "looked up, absent").
const UNKNOWN: usize = usize::MAX;

/// The system library's own `name` (not this file's shim), cached.
fn system_fn(cache: &AtomicUsize, lib: &std::ffi::CStr, name: &std::ffi::CStr) -> usize {
    let v = cache.load(Ordering::Acquire);
    if v != UNKNOWN {
        return v;
    }
    // SAFETY: NUL-terminated names; dlsym on the library's own handle only
    // searches that library, so it never returns the shims below.
    let f = unsafe {
        let h = dlopen(lib.as_ptr(), RTLD_NOW);
        if h.is_null() { 0 } else { dlsym(h, name.as_ptr()) as usize }
    };
    cache.store(f, Ordering::Release);
    f
}

fn set_errno(e: c_int) {
    // SAFETY: bionic's thread-local errno.
    unsafe { *__errno() = e };
}

static GETIFADDRS: AtomicUsize = AtomicUsize::new(UNKNOWN);
static FREEIFADDRS: AtomicUsize = AtomicUsize::new(UNKNOWN);
static READLINKAT: AtomicUsize = AtomicUsize::new(UNKNOWN);
static DL_ITERATE_PHDR: AtomicUsize = AtomicUsize::new(UNKNOWN);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn getifaddrs(ifap: *mut *mut c_void) -> c_int {
    let f = system_fn(&GETIFADDRS, c"libc.so", c"getifaddrs");
    if f == 0 {
        set_errno(ENOSYS);
        return -1;
    }
    // SAFETY: the system's getifaddrs, same signature.
    unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(*mut *mut c_void) -> c_int>(f)(ifap) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeifaddrs(ifa: *mut c_void) {
    let f = system_fn(&FREEIFADDRS, c"libc.so", c"freeifaddrs");
    if f != 0 && !ifa.is_null() {
        // SAFETY: the system's freeifaddrs on a list its getifaddrs returned.
        unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(*mut c_void)>(f)(ifa) }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn readlinkat(dirfd: c_int, path: *const c_char, buf: *mut c_char, size: usize) -> isize {
    let f = system_fn(&READLINKAT, c"libc.so", c"readlinkat");
    // SAFETY: forwarding the caller's arguments unchanged.
    unsafe {
        if f != 0 {
            let real: unsafe extern "C" fn(c_int, *const c_char, *mut c_char, usize) -> isize = std::mem::transmute(f);
            return real(dirfd, path, buf, size);
        }
        // Android 4.4: plain readlink covers the cwd-relative and absolute cases.
        if dirfd == AT_FDCWD || (!path.is_null() && *path == b'/' as c_char) {
            return readlink(path, buf, size);
        }
    }
    set_errno(ENOSYS);
    -1
}

type PhdrCallback = Option<unsafe extern "C" fn(*mut c_void, usize, *mut c_void) -> c_int>;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dl_iterate_phdr(cb: PhdrCallback, data: *mut c_void) -> c_int {
    let f = system_fn(&DL_ITERATE_PHDR, c"libdl.so", c"dl_iterate_phdr");
    let f = if f != 0 { f } else { system_fn(&DL_ITERATE_PHDR, c"libc.so", c"dl_iterate_phdr") };
    if f == 0 {
        // Android 4.4 on ARM: nothing to iterate (only used to unwind after a
        // panic, which then aborts instead).
        return 0;
    }
    // SAFETY: the system's dl_iterate_phdr, same signature.
    unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(PhdrCallback, *mut c_void) -> c_int>(f)(cb, data) }
}
