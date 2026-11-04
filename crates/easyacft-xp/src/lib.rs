//! Runs an EasyAcft aircraft inside X-Plane: dataref bindings, published values, commands, reload.

pub mod live;
pub mod panel;
pub mod xplm;

use easyacft_parts::Aircraft;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A dataref or command handle.
pub type Ref = usize;

/// What the binder needs from X-Plane; `xplm::Xplm` is the real one.
pub trait Sim {
    fn find(&self, name: &str) -> Option<Ref>;
    fn get(&self, r: Ref, index: Option<usize>) -> f64;
    fn set(&self, r: Ref, index: Option<usize>, v: f64);
    fn log(&self, msg: &str);
}

/// `"sim/x/y[3]"` into the name and index.
pub fn split_index(name: &str) -> (&str, Option<usize>) {
    match name.strip_suffix(']').and_then(|n| n.rsplit_once('[')) {
        Some((base, i)) => match i.parse() {
            Ok(i) => (base, Some(i)),
            Err(_) => (name, None),
        },
        None => (name, None),
    }
}

struct Bound {
    dataref: Ref,
    index: Option<usize>,
    slot: usize,
    to_easyacft: bool,
    factor: f64,
    offset: f64,
}

pub struct Runner {
    pub aircraft: Aircraft,
    pub dir: PathBuf,
    bound: Vec<Bound>,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    reload_s: f64,
    time_ref: Option<Ref>,
    last_time: Option<f64>,
    forces: Vec<(usize, [f64; 3], [f64; 3], f64, bool)>,
    force_refs: Option<([Ref; 6], [Ref; 6], Ref)>,
    pub live: live::Server,
    kinds: Vec<(String, KindHook)>,
}

/// A custom-kind handler kept across reloads.
pub type KindHook = std::rc::Rc<std::cell::RefCell<dyn FnMut(&mut easyacft_parts::custom::Part<'_>)>>;

const PLUG: [&str; 6] = ["fside_plug_acf", "fnrml_plug_acf", "faxil_plug_acf", "L_plug_acf", "M_plug_acf", "N_plug_acf"];
const PROP: [&str; 6] = ["fside_prop", "fnrml_prop", "faxil_prop", "L_prop", "M_prop", "N_prop"];

fn stamp(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Runner {
    pub fn load(dir: &Path, sim: &dyn Sim) -> Option<Self> {
        match Aircraft::load(dir) {
            Ok((aircraft, report)) => {
                if !report.diagnostics.is_empty() {
                    sim.log(&report.render());
                }
                let mut r = Runner { aircraft, dir: dir.to_path_buf(), bound: Vec::new(), stamps: Vec::new(), reload_s: 0.0, time_ref: sim.find("sim/time/total_flight_time_sec"), last_time: None, forces: Vec::new(), force_refs: None, live: live::Server::start(), kinds: Vec::new() };
                r.bind(sim);
                Some(r)
            }
            Err(report) => {
                sim.log(&format!("EasyAcft: {} has errors:\n{}", dir.display(), report.render()));
                None
            }
        }
    }

    fn bind(&mut self, sim: &dyn Sim) {
        self.bound.clear();
        for b in &self.aircraft.bindings {
            let (name, index) = split_index(&b.dataref);
            let (Some(dataref), Some(slot)) = (sim.find(name), self.aircraft.slot(&b.path)) else {
                sim.log(&format!("EasyAcft: binding {} = {} skipped (no such dataref or value)", b.dataref, b.path));
                continue;
            };
            self.bound.push(Bound { dataref, index, slot, to_easyacft: b.write, factor: if b.factor == 0.0 { 1.0 } else { b.factor }, offset: b.offset });
        }
        self.stamps = self.aircraft.files.iter().map(|f| (PathBuf::from(f), stamp(Path::new(f)))).collect();
        self.forces = easyacft_parts::engine::forces(&self.aircraft.checked);
        let find = |n: &str| sim.find(&format!("sim/flightmodel/forces/{n}"));
        let plug: Option<Vec<Ref>> = PLUG.iter().map(|n| find(n)).collect();
        let prop: Option<Vec<Ref>> = PROP.iter().map(|n| find(n)).collect();
        let ov = sim.find("sim/operation/override/override_engine_forces");
        self.force_refs = match (plug, prop, ov) {
            (Some(a), Some(b), Some(o)) if !self.forces.is_empty() => Some((a.try_into().unwrap(), b.try_into().unwrap(), o)),
            _ => None,
        };
    }

    fn apply_forces(&self, sim: &dyn Sim) {
        let Some((plug, prop, ov)) = self.force_refs else { return };
        let pick = |replace: bool| -> Vec<([f64; 3], [f64; 3], f64)> { self.forces.iter().filter(|f| f.4 == replace).map(|f| (f.1, f.2, self.aircraft.slots[f.0] * f.3)).collect() };
        let replaced = pick(true);
        if !replaced.is_empty() {
            sim.set(ov, None, 1.0);
            for (r, v) in prop.iter().zip(easyacft_parts::engine::sum_forces(&replaced)) {
                sim.set(*r, None, v);
            }
        }
        let added = pick(false);
        if !added.is_empty() {
            for (r, v) in plug.iter().zip(easyacft_parts::engine::sum_forces(&added)) {
                sim.set(*r, None, sim.get(*r, None) + v);
            }
        }
    }

    /// One X-Plane frame; `frame_s` is used only when sim time is unavailable.
    pub fn frame(&mut self, sim: &dyn Sim, frame_s: f64) {
        let dt = match self.time_ref.map(|r| sim.get(r, None)) {
            Some(t) => {
                let d = self.last_time.map_or(0.0, |l| t - l);
                self.last_time = Some(t);
                if d < 0.0 || d > 1.0 { 0.0 } else { d }
            }
            None => frame_s.clamp(0.0, 0.5),
        };
        for b in self.bound.iter().filter(|b| b.to_easyacft) {
            self.aircraft.slots[b.slot] = sim.get(b.dataref, b.index) * b.factor + b.offset;
        }
        if dt > 0.0 {
            self.aircraft.step(dt);
        }
        for b in self.bound.iter().filter(|b| !b.to_easyacft) {
            sim.set(b.dataref, b.index, (self.aircraft.slots[b.slot] - b.offset) / b.factor);
        }
        self.apply_forces(sim);
        self.live.serve(&mut self.aircraft, frame_s);
        self.reload_s += frame_s.max(0.0);
        if self.reload_s >= 1.0 {
            self.reload_s = 0.0;
            if self.stamps.iter().any(|(p, s)| stamp(p) != *s) {
                self.reload(sim);
            }
        }
    }

    /// Steps every `[[custom.<kind>]]` part with `h`, now and after each reload.
    pub fn on_kind(&mut self, kind: &str, h: KindHook) -> usize {
        let h2 = h.clone();
        self.kinds.push((kind.to_owned(), h));
        self.aircraft.on_kind(kind, move |p| (h2.borrow_mut())(p))
    }

    /// Reloads the files, keeping every value whose name still exists.
    pub fn reload(&mut self, sim: &dyn Sim) {
        match Aircraft::load(&self.dir) {
            Ok((mut fresh, report)) => {
                if !report.diagnostics.is_empty() {
                    sim.log(&report.render());
                }
                for (i, (name, _)) in self.aircraft.checked.slots.names.iter().enumerate() {
                    fresh.set(name, self.aircraft.slots[i]);
                }
                self.aircraft = fresh;
                for (k, h) in &self.kinds {
                    let h = h.clone();
                    self.aircraft.on_kind(k, move |p| (h.borrow_mut())(p));
                }
                self.bind(sim);
                sim.log("EasyAcft: reloaded");
            }
            Err(report) => {
                sim.log(&format!("EasyAcft: not reloaded, the files have errors:\n{}", report.render()));
                self.stamps = self.aircraft.files.iter().map(|f| (PathBuf::from(f), stamp(Path::new(f)))).collect();
            }
        }
    }

    /// `easyacft/<id>/<value>` for every value, the names the plugin publishes.
    pub fn published(&self) -> Vec<(String, usize)> {
        self.aircraft.names().enumerate().map(|(i, n)| (format!("easyacft/{}", n.replace('.', "/")), i)).collect()
    }

    /// Cockpit switch commands: `easyacft/<id>/toggle`, `/on`, `/off`.
    pub fn commands(&self) -> Vec<(String, usize, f64)> {
        let mut out = Vec::new();
        for p in self.aircraft.checked.parts.iter().filter(|p| p.domain == "cockpit" && p.kind == "switch") {
            let Some(slot) = self.aircraft.slot(&format!("{}.on", p.id)) else { continue };
            out.push((format!("easyacft/{}/toggle", p.id), slot, -1.0));
            out.push((format!("easyacft/{}/on", p.id), slot, 1.0));
            out.push((format!("easyacft/{}/off", p.id), slot, 0.0));
        }
        out
    }

    pub fn command(&mut self, slot: usize, value: f64) {
        let v = &mut self.aircraft.slots[slot];
        *v = if value < 0.0 { (*v == 0.0) as u8 as f64 } else { value };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mock {
        refs: Vec<String>,
        values: RefCell<HashMap<(usize, Option<usize>), f64>>,
        log: RefCell<Vec<String>>,
    }
    impl Sim for Mock {
        fn find(&self, n: &str) -> Option<Ref> {
            self.refs.iter().position(|r| r == n)
        }
        fn get(&self, r: Ref, i: Option<usize>) -> f64 {
            *self.values.borrow().get(&(r, i)).unwrap_or(&0.0)
        }
        fn set(&self, r: Ref, i: Option<usize>, v: f64) {
            self.values.borrow_mut().insert((r, i), v);
        }
        fn log(&self, m: &str) {
            self.log.borrow_mut().push(m.to_owned());
        }
    }

    #[test]
    fn bindings_move_values_both_ways_on_sim_time() {
        let dir = std::env::temp_dir().join(format!("easyacft-xp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("aircraft.toml"),
            "[bindings]\n\"sim/volts[1]\" = \"dc.voltage\"\n\"sim/bat_switch\" = { to = \"sw.on\" }\n\"sim/missing\" = \"dc.voltage\"\n[[cockpit.switch]]\nid = \"sw\"\n[[electrical.bus]]\nid = \"dc\"\nnominal = \"28 V\"\n[[electrical.battery]]\nid = \"b\"\nbus = \"hot\"\nvoltage = \"24 V\"\ncapacity = \"20 Ah\"\n[[electrical.bus]]\nid = \"hot\"\nnominal = \"28 V\"\n[[electrical.contactor]]\nid = \"c\"\nbetween = [\"hot\", \"dc\"]\nclose_when = \"sw.on\"\n",
        )
        .unwrap();
        let sim = Mock { refs: vec!["sim/time/total_flight_time_sec".into(), "sim/volts".into(), "sim/bat_switch".into()], ..Default::default() };
        let mut r = Runner::load(&dir, &sim).expect("loads");
        assert!(sim.log.borrow().iter().any(|l| l.contains("sim/missing")));
        sim.set(2, None, 1.0);
        for k in 0..30 {
            sim.set(0, None, k as f64 * 0.05);
            r.frame(&sim, 0.05);
        }
        assert!(sim.get(1, Some(1)) > 20.0, "dc voltage written to sim/volts[1]");
        sim.set(2, None, 0.0);
        for k in 30..40 {
            sim.set(0, None, k as f64 * 0.05);
            r.frame(&sim, 0.05);
        }
        assert!(sim.get(1, Some(1)) < 1.0, "switch read from X-Plane opened the contactor");
        assert!(r.published().iter().any(|(n, _)| n == "easyacft/dc/voltage"));
        let toggle = r.commands().into_iter().find(|(n, _, _)| n == "easyacft/sw/toggle").unwrap();
        r.command(toggle.1, toggle.2);
        assert_eq!(r.aircraft.get("sw.on"), Some(1.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forces_add_to_the_plugin_force_datarefs() {
        let dir = std::env::temp_dir().join(format!("easyacft-xpf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aircraft.toml"), "[[logic.flag]]
id = \"on\"
when = \"true\"
[[output.force]]
id = \"thrust\"
force = \"if(on.on, 1000 N, 0 N)\"
x = \"5 m\"
y = \"-2 m\"
z = \"0 m\"
").unwrap();
        let mut refs: Vec<String> = vec!["sim/time/total_flight_time_sec".into()];
        refs.extend(PLUG.iter().chain(PROP.iter()).map(|n| format!("sim/flightmodel/forces/{n}")));
        refs.push("sim/operation/override/override_engine_forces".into());
        let sim = Mock { refs, ..Default::default() };
        let mut r = Runner::load(&dir, &sim).expect("loads");
        for k in 0..3 {
            sim.set(0, None, k as f64 * 0.1);
            for i in 1..=6 {
                sim.set(i, None, 0.0);
            }
            r.frame(&sim, 0.1);
        }
        assert!((sim.get(3, None) + 1000.0).abs() < 1e-6, "faxil_plug_acf {}", sim.get(3, None));
        assert!(sim.get(5, None) > 0.0, "thrust below the cg pitches up");
        assert!(sim.get(6, None) < 0.0, "thrust right of the cg yaws left");
        assert_eq!(sim.get(13, None), 0.0, "X-Plane keeps its own engines");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn indexes_parse() {
        assert_eq!(split_index("sim/a/b[12]"), ("sim/a/b", Some(12)));
        assert_eq!(split_index("sim/a/b"), ("sim/a/b", None));
        assert_eq!(split_index("sim/a[x]"), ("sim/a[x]", None));
    }
}
