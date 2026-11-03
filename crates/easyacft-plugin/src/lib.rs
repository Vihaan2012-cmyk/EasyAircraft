//! X-Plane plugin: runs `<aircraft folder>/easyacft/aircraft.toml`. Build, rename to `easyacft.xpl`.

use easyacft_xp::xplm::{Api, CommandRef, DataRef, Xplm};
use easyacft_xp::{Runner, Sim};
use std::cell::RefCell;
use std::ffi::{c_char, c_double, c_float, c_int, c_void, CStr, CString};
use std::path::PathBuf;

struct State {
    sim: Xplm,
    runner: Option<Runner>,
    published: Vec<DataRef>,
    commands: Vec<(CommandRef, usize, f64)>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn copy(dst: *mut c_char, s: &str) {
    let c = CString::new(s).unwrap();
    unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), dst, c.as_bytes_with_nul().len()) }
}

/// # Safety
/// Called by X-Plane with 256-byte buffers.
#[no_mangle]
pub unsafe extern "C" fn XPluginStart(name: *mut c_char, sig: *mut c_char, desc: *mut c_char) -> c_int {
    copy(name, "EasyAcft");
    copy(sig, "org.easyacft.plugin");
    copy(desc, "Aircraft systems from the easyacft folder");
    let Some(api) = Api::load() else { return 0 };
    STATE.with(|s| *s.borrow_mut() = Some(State { sim: Xplm::new(api), runner: None, published: Vec::new(), commands: Vec::new() }));
    1
}

#[no_mangle]
pub extern "C" fn XPluginStop() {
    STATE.with(|s| *s.borrow_mut() = None);
}

fn aircraft_dir(api: &Api) -> PathBuf {
    let mut file = [0 as c_char; 256];
    let mut path = [0 as c_char; 512];
    unsafe { (api.GetNthAircraftModel)(0, file.as_mut_ptr(), path.as_mut_ptr()) };
    let p = unsafe { CStr::from_ptr(path.as_ptr()) }.to_string_lossy().into_owned();
    PathBuf::from(p).parent().map(|d| d.join("easyacft")).unwrap_or_default()
}

unsafe extern "C" fn read(refcon: *mut c_void) -> c_double {
    STATE.with(|s| s.borrow().as_ref().and_then(|st| st.runner.as_ref()).and_then(|r| r.aircraft.slots.get(refcon as usize).copied()).unwrap_or(0.0))
}

unsafe extern "C" fn write(refcon: *mut c_void, v: c_double) {
    STATE.with(|s| {
        if let Some(slot) = s.borrow_mut().as_mut().and_then(|st| st.runner.as_mut()).and_then(|r| r.aircraft.slots.get_mut(refcon as usize)) {
            *slot = v;
        }
    })
}

unsafe extern "C" fn readf(refcon: *mut c_void) -> c_float {
    read(refcon) as c_float
}

unsafe extern "C" fn writef(refcon: *mut c_void, v: c_float) {
    write(refcon, v as c_double)
}

unsafe extern "C" fn on_command(_: CommandRef, phase: c_int, refcon: *mut c_void) -> c_int {
    if phase == 0 {
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                if let Some(&(_, slot, value)) = st.commands.get(refcon as usize) {
                    if let Some(r) = st.runner.as_mut() {
                        r.command(slot, value);
                    }
                }
            }
        });
    }
    1
}

unsafe extern "C" fn flight_loop(since_last: c_float, _: c_float, _: c_int, _: *mut c_void) -> c_float {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            if let Some(r) = st.runner.as_mut() {
                r.frame(&st.sim, since_last as f64);
            }
        }
    });
    -1.0
}

/// # Safety
/// Called by X-Plane.
#[no_mangle]
pub unsafe extern "C" fn XPluginEnable() -> c_int {
    STATE.with(|s| {
        let mut b = s.borrow_mut();
        let Some(st) = b.as_mut() else { return 0 };
        let dir = aircraft_dir(&st.sim.api);
        st.runner = Runner::load(&dir, &st.sim);
        let Some(r) = st.runner.as_mut() else { return 0 };
        if let Ok(text) = std::fs::read_to_string(dir.join("persistent.txt")) {
            r.aircraft.restore_persistent(&text);
        }
        let r = &*r;
        st.sim.log(&format!("EasyAcft: {} loaded from {}", r.aircraft.name, dir.display()));
        for (name, slot) in r.published() {
            let n = CString::new(name).unwrap();
            let none = std::ptr::null();
            let d = (st.sim.api.RegisterDataAccessor)(n.as_ptr(), 2 | 4, 1, none, none, Some(readf), Some(writef), Some(read), Some(write), none, none, none, none, none, none, slot as *mut c_void, slot as *mut c_void);
            st.published.push(d);
        }
        for (k, (name, slot, value)) in r.commands().into_iter().enumerate() {
            let n = CString::new(name).unwrap();
            let c = (st.sim.api.CreateCommand)(n.as_ptr(), c"EasyAcft cockpit switch".as_ptr());
            (st.sim.api.RegisterCommandHandler)(c, on_command, 1, k as *mut c_void);
            st.commands.push((c, slot, value));
        }
        (st.sim.api.RegisterFlightLoopCallback)(flight_loop, -1.0, std::ptr::null_mut());
        1
    })
}

/// # Safety
/// Called by X-Plane.
#[no_mangle]
pub unsafe extern "C" fn XPluginDisable() {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            (st.sim.api.UnregisterFlightLoopCallback)(flight_loop, std::ptr::null_mut());
            for d in st.published.drain(..) {
                (st.sim.api.UnregisterDataAccessor)(d);
            }
            for (k, (c, _, _)) in st.commands.drain(..).enumerate() {
                (st.sim.api.UnregisterCommandHandler)(c, on_command, 1, k as *mut c_void);
            }
            if let Some(r) = st.runner.take() {
                let _ = std::fs::write(r.dir.join("persistent.txt"), r.aircraft.persistent());
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn XPluginReceiveMessage(_from: c_int, _msg: c_int, _param: *mut c_void) {}
