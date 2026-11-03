//! The X-Plane SDK functions EasyAcft uses, looked up at run time from the running sim.

use crate::{Ref, Sim};
use std::cell::RefCell;
use std::ffi::{c_char, c_double, c_float, c_int, c_void, CString};

pub type DataRef = *mut c_void;
pub type CommandRef = *mut c_void;
pub type GetD = unsafe extern "C" fn(*mut c_void) -> c_double;
pub type SetD = unsafe extern "C" fn(*mut c_void, c_double);
pub type GetF = unsafe extern "C" fn(*mut c_void) -> c_float;
pub type SetF = unsafe extern "C" fn(*mut c_void, c_float);
pub type FlightLoop = unsafe extern "C" fn(c_float, c_float, c_int, *mut c_void) -> c_float;
pub type CommandFn = unsafe extern "C" fn(CommandRef, c_int, *mut c_void) -> c_int;
type Any = *const c_void;

#[allow(non_snake_case)]
pub struct Api {
    pub FindDataRef: unsafe extern "C" fn(*const c_char) -> DataRef,
    pub GetDataRefTypes: unsafe extern "C" fn(DataRef) -> c_int,
    pub GetDatai: unsafe extern "C" fn(DataRef) -> c_int,
    pub SetDatai: unsafe extern "C" fn(DataRef, c_int),
    pub GetDataf: unsafe extern "C" fn(DataRef) -> c_float,
    pub SetDataf: unsafe extern "C" fn(DataRef, c_float),
    pub GetDatad: unsafe extern "C" fn(DataRef) -> c_double,
    pub SetDatad: unsafe extern "C" fn(DataRef, c_double),
    pub GetDatavf: unsafe extern "C" fn(DataRef, *mut c_float, c_int, c_int) -> c_int,
    pub SetDatavf: unsafe extern "C" fn(DataRef, *const c_float, c_int, c_int),
    pub GetDatavi: unsafe extern "C" fn(DataRef, *mut c_int, c_int, c_int) -> c_int,
    pub SetDatavi: unsafe extern "C" fn(DataRef, *const c_int, c_int, c_int),
    #[allow(clippy::type_complexity)]
    pub RegisterDataAccessor: unsafe extern "C" fn(*const c_char, c_int, c_int, Any, Any, Option<GetF>, Option<SetF>, Option<GetD>, Option<SetD>, Any, Any, Any, Any, Any, Any, *mut c_void, *mut c_void) -> DataRef,
    pub UnregisterDataAccessor: unsafe extern "C" fn(DataRef),
    pub RegisterFlightLoopCallback: unsafe extern "C" fn(FlightLoop, c_float, *mut c_void),
    pub UnregisterFlightLoopCallback: unsafe extern "C" fn(FlightLoop, *mut c_void),
    pub CreateCommand: unsafe extern "C" fn(*const c_char, *const c_char) -> CommandRef,
    pub RegisterCommandHandler: unsafe extern "C" fn(CommandRef, CommandFn, c_int, *mut c_void),
    pub UnregisterCommandHandler: unsafe extern "C" fn(CommandRef, CommandFn, c_int, *mut c_void),
    pub DebugString: unsafe extern "C" fn(*const c_char),
    pub GetNthAircraftModel: unsafe extern "C" fn(c_int, *mut c_char, *mut c_char),
}

#[cfg(windows)]
unsafe fn symbol(name: &str) -> *const c_void {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }
    let module = GetModuleHandleA(c"XPLM_64.dll".as_ptr());
    if module.is_null() {
        return std::ptr::null();
    }
    let n = CString::new(name).unwrap();
    GetProcAddress(module, n.as_ptr())
}

#[cfg(not(windows))]
unsafe fn symbol(name: &str) -> *const c_void {
    extern "C" {
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *const c_void;
    }
    #[cfg(target_os = "macos")]
    const DEFAULT: *mut c_void = -2isize as *mut c_void;
    #[cfg(not(target_os = "macos"))]
    const DEFAULT: *mut c_void = std::ptr::null_mut();
    let n = CString::new(name).unwrap();
    dlsym(DEFAULT, n.as_ptr())
}

impl Api {
    /// # Safety
    /// Call only inside X-Plane, where XPLM is loaded.
    pub unsafe fn load() -> Option<Self> {
        macro_rules! get {
            ($n:literal) => {{
                let p = symbol(concat!("XPLM", $n));
                if p.is_null() {
                    return None;
                }
                std::mem::transmute::<*const c_void, _>(p)
            }};
        }
        Some(Api {
            FindDataRef: get!("FindDataRef"),
            GetDataRefTypes: get!("GetDataRefTypes"),
            GetDatai: get!("GetDatai"),
            SetDatai: get!("SetDatai"),
            GetDataf: get!("GetDataf"),
            SetDataf: get!("SetDataf"),
            GetDatad: get!("GetDatad"),
            SetDatad: get!("SetDatad"),
            GetDatavf: get!("GetDatavf"),
            SetDatavf: get!("SetDatavf"),
            GetDatavi: get!("GetDatavi"),
            SetDatavi: get!("SetDatavi"),
            RegisterDataAccessor: get!("RegisterDataAccessor"),
            UnregisterDataAccessor: get!("UnregisterDataAccessor"),
            RegisterFlightLoopCallback: get!("RegisterFlightLoopCallback"),
            UnregisterFlightLoopCallback: get!("UnregisterFlightLoopCallback"),
            CreateCommand: get!("CreateCommand"),
            RegisterCommandHandler: get!("RegisterCommandHandler"),
            UnregisterCommandHandler: get!("UnregisterCommandHandler"),
            DebugString: get!("DebugString"),
            GetNthAircraftModel: get!("GetNthAircraftModel"),
        })
    }
}

const INT: c_int = 1;
const FLOAT: c_int = 2;
const DOUBLE: c_int = 4;
const FLOAT_ARRAY: c_int = 8;
const INT_ARRAY: c_int = 16;

pub struct Xplm {
    pub api: Api,
    refs: RefCell<Vec<(DataRef, c_int)>>,
}

impl Xplm {
    pub fn new(api: Api) -> Self {
        Self { api, refs: RefCell::new(Vec::new()) }
    }
}

impl Sim for Xplm {
    fn find(&self, name: &str) -> Option<Ref> {
        let n = CString::new(name).ok()?;
        let d = unsafe { (self.api.FindDataRef)(n.as_ptr()) };
        if d.is_null() {
            return None;
        }
        let t = unsafe { (self.api.GetDataRefTypes)(d) };
        let mut refs = self.refs.borrow_mut();
        refs.push((d, t));
        Some(refs.len() - 1)
    }

    fn get(&self, r: Ref, index: Option<usize>) -> f64 {
        let Some(&(d, t)) = self.refs.borrow().get(r) else { return 0.0 };
        unsafe {
            match index {
                Some(i) if t & FLOAT_ARRAY != 0 => {
                    let mut v = 0.0f32;
                    (self.api.GetDatavf)(d, &mut v, i as c_int, 1);
                    v as f64
                }
                Some(i) if t & INT_ARRAY != 0 => {
                    let mut v = 0;
                    (self.api.GetDatavi)(d, &mut v, i as c_int, 1);
                    v as f64
                }
                _ if t & DOUBLE != 0 => (self.api.GetDatad)(d),
                _ if t & FLOAT != 0 => (self.api.GetDataf)(d) as f64,
                _ if t & INT != 0 => (self.api.GetDatai)(d) as f64,
                _ => 0.0,
            }
        }
    }

    fn set(&self, r: Ref, index: Option<usize>, v: f64) {
        let Some(&(d, t)) = self.refs.borrow().get(r) else { return };
        unsafe {
            match index {
                Some(i) if t & FLOAT_ARRAY != 0 => (self.api.SetDatavf)(d, &(v as f32), i as c_int, 1),
                Some(i) if t & INT_ARRAY != 0 => (self.api.SetDatavi)(d, &(v.round() as c_int), i as c_int, 1),
                _ if t & DOUBLE != 0 => (self.api.SetDatad)(d, v),
                _ if t & FLOAT != 0 => (self.api.SetDataf)(d, v as f32),
                _ if t & INT != 0 => (self.api.SetDatai)(d, v.round() as c_int),
                _ => {}
            }
        }
    }

    fn log(&self, msg: &str) {
        let text = if msg.ends_with('\n') { msg.to_owned() } else { format!("{msg}\n") };
        if let Ok(c) = CString::new(text.replace('\0', " ")) {
            unsafe { (self.api.DebugString)(c.as_ptr()) }
        }
    }
}
