//! EasyAcft C API (`include/easyacft.h`).

use easyacft_parts::Aircraft;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

pub type StepFn = extern "C" fn(*mut Handle, f64, *mut c_void);

pub struct Handle {
    pub aircraft: Aircraft,
    names: Vec<CString>,
    hooks: Vec<(StepFn, *mut c_void, Option<usize>)>,
    log: Vec<CString>,
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn give(s: String) -> *mut c_char {
    CString::new(s.replace('\0', " ")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `dir` NUL-terminated; `errors` null or writable.
#[no_mangle]
pub unsafe extern "C" fn easyacft_load(dir: *const c_char, errors: *mut *mut c_char) -> *mut Handle {
    guard(std::ptr::null_mut(), || {
        let report = |s: String| {
            if !errors.is_null() {
                *errors = give(s);
            }
        };
        if dir.is_null() {
            report("no folder given".into());
            return std::ptr::null_mut();
        }
        let dir = CStr::from_ptr(dir).to_string_lossy().into_owned();
        match Aircraft::load(Path::new(&dir)) {
            Ok((aircraft, _)) => {
                let names = aircraft.names().map(|n| CString::new(n).unwrap_or_default()).collect();
                Box::into_raw(Box::new(Handle { aircraft, names, hooks: Vec::new(), log: Vec::new() }))
            }
            Err(r) => {
                report(r.render());
                std::ptr::null_mut()
            }
        }
    })
}

/// # Safety
/// `a` from `easyacft_load`, unused afterwards.
#[no_mangle]
pub unsafe extern "C" fn easyacft_free(a: *mut Handle) {
    if !a.is_null() {
        drop(Box::from_raw(a));
    }
}

/// # Safety
/// `s` from this library.
#[no_mangle]
pub unsafe extern "C" fn easyacft_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_step(a: *mut Handle, dt: f64) {
    let Some(h) = a.as_mut() else { return };
    let hooks = h.hooks.clone();
    guard((), || h.aircraft.step(dt));
    for (f, user, gate) in hooks {
        if gate.is_none_or(|g| (&(*a).aircraft.slots)[g] != 0.0) {
            f(a, dt, user);
        }
    }
}

/// # Safety
/// `a` live, `name` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn easyacft_find(a: *const Handle, name: *const c_char) -> c_int {
    let (Some(h), false) = (a.as_ref(), name.is_null()) else { return -1 };
    let n = CStr::from_ptr(name).to_string_lossy();
    h.aircraft.slot(&n).map_or(-1, |i| i as c_int)
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_get(a: *const Handle, handle: c_int) -> f64 {
    a.as_ref().and_then(|h| h.aircraft.slots.get(handle as usize).copied()).unwrap_or(0.0)
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_set(a: *mut Handle, handle: c_int, value: f64) {
    if let Some(v) = a.as_mut().and_then(|h| h.aircraft.slots.get_mut(handle as usize)) {
        *v = value;
    }
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_count(a: *const Handle) -> c_int {
    a.as_ref().map_or(0, |h| h.names.len() as c_int)
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_name(a: *const Handle, handle: c_int) -> *const c_char {
    a.as_ref().and_then(|h| h.names.get(handle as usize)).map_or(std::ptr::null(), |s| s.as_ptr())
}

/// # Safety
/// `a` live; `buf` null or `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn easyacft_snapshot(a: *const Handle, buf: *mut u8, len: usize) -> usize {
    let Some(h) = a.as_ref() else { return 0 };
    let bytes = h.aircraft.snapshot().to_bytes();
    if !buf.is_null() && len >= bytes.len() {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
    }
    bytes.len()
}

/// # Safety
/// `a` live; `buf` `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn easyacft_restore(a: *mut Handle, buf: *const u8, len: usize) -> c_int {
    let (Some(h), false) = (a.as_mut(), buf.is_null()) else { return 0 };
    let bytes = std::slice::from_raw_parts(buf, len);
    easyacft_core::Snapshot::from_bytes(bytes).is_some_and(|s| h.aircraft.restore(&s)) as c_int
}

/// # Safety
/// `a` live; `user` valid while the aircraft lives.
#[no_mangle]
pub unsafe extern "C" fn easyacft_on_step(a: *mut Handle, f: StepFn, user: *mut c_void) {
    if let Some(h) = a.as_mut() {
        h.hooks.push((f, user, None));
    }
}

/// # Safety
/// `a` live, `computer` NUL-terminated; `user` valid while the aircraft lives.
#[no_mangle]
pub unsafe extern "C" fn easyacft_on_computer_step(a: *mut Handle, computer: *const c_char, f: StepFn, user: *mut c_void) -> c_int {
    let (Some(h), false) = (a.as_mut(), computer.is_null()) else { return 0 };
    let id = CStr::from_ptr(computer).to_string_lossy();
    match h.aircraft.slot(&format!("{id}.running")) {
        Some(slot) => {
            h.hooks.push((f, user, Some(slot)));
            1
        }
        None => 0,
    }
}

/// # Safety
/// `a` live.
#[no_mangle]
pub unsafe extern "C" fn easyacft_log_count(a: *mut Handle) -> c_int {
    let Some(h) = a.as_mut() else { return 0 };
    h.log = h.aircraft.log.iter().map(|e| CString::new(format!("{:.1} {} {}", e.time, e.what, if e.on_ground { "ground" } else { "air" })).unwrap_or_default()).collect();
    h.log.len() as c_int
}

/// # Safety
/// `a` live; call `easyacft_log_count` first.
#[no_mangle]
pub unsafe extern "C" fn easyacft_log_entry(a: *const Handle, i: c_int) -> *const c_char {
    a.as_ref().and_then(|h| h.log.get(i as usize)).map_or(std::ptr::null(), |s| s.as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn count(_: *mut Handle, _: f64, user: *mut c_void) {
        unsafe { *(user as *mut u32) += 1 }
    }

    #[test]
    fn the_c_api_runs_the_example() {
        let dir = CString::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/twin-jet")).unwrap();
        unsafe {
            let mut err = std::ptr::null_mut();
            let a = easyacft_load(dir.as_ptr(), &mut err);
            assert!(!a.is_null(), "{}", if err.is_null() { String::new() } else { CStr::from_ptr(err).to_string_lossy().into_owned() });
            let sw = easyacft_find(a, c"bat_sw.on".as_ptr());
            let v = easyacft_find(a, c"dc_ess.voltage".as_ptr());
            assert!(sw >= 0 && v >= 0 && easyacft_find(a, c"nope".as_ptr()) == -1);
            assert_eq!(CStr::from_ptr(easyacft_name(a, v)).to_str().unwrap(), "dc_ess.voltage");
            let mut calls = 0u32;
            easyacft_on_step(a, count, &mut calls as *mut u32 as *mut c_void);
            easyacft_set(a, sw, 1.0);
            for _ in 0..60 {
                easyacft_step(a, 1.0 / 60.0);
            }
            assert_eq!(calls, 60);
            assert!(easyacft_get(a, v) > 20.0);
            let n = easyacft_snapshot(a, std::ptr::null_mut(), 0);
            let mut buf = vec![0u8; n];
            assert_eq!(easyacft_snapshot(a, buf.as_mut_ptr(), n), n);
            easyacft_set(a, sw, 0.0);
            assert_eq!(easyacft_restore(a, buf.as_ptr(), n), 1);
            assert_eq!(easyacft_get(a, sw), 1.0);
            easyacft_free(a);
            let bad = CString::new("/no/such/aircraft").unwrap();
            assert!(easyacft_load(bad.as_ptr(), &mut err).is_null());
            assert!(CStr::from_ptr(err).to_string_lossy().contains("cannot read"));
            easyacft_free_string(err);
        }
    }
}
