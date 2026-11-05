//! Imports an X-Plane .acf or an MSFS aircraft into a starting set of EasyAcft files.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

mod afdx;
mod catalogue;
mod code;
mod deep_parts;
mod fbw_links;
mod msfs_fuel;
pub mod plugin;

const LB: f64 = 0.453_592_37;
const FT: f64 = 0.3048;
const LBF: f64 = 4.448_221_6;
const HP: f64 = 745.7;
const GAL: f64 = 3.785_411_784;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineKind {
    Jet,
    Turboprop,
    Piston,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Engine {
    pub kind: EngineKind,
    pub thrust_n: f64,
    pub power_w: f64,
    pub rpm: f64,
    pub prop_diameter_m: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tank {
    pub name: String,
    pub capacity_kg: f64,
    pub arm_m: f64,
    pub lateral_m: f64,
}

/// What an import found, before it is written as files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spec {
    pub name: String,
    pub source: String,
    pub batteries: usize,
    pub battery_voltage: f64,
    pub battery_ah: f64,
    pub generators: usize,
    pub generator_voltage: f64,
    pub generator_w: f64,
    pub buses: usize,
    pub inverters: usize,
    pub engines: Vec<Engine>,
    pub tanks: Vec<Tank>,
    pub empty_kg: f64,
    pub empty_arm_m: f64,
    pub retracting_gear: Option<f64>,
    pub hydraulic_psi: Option<f64>,
    pub notes: Vec<String>,
    /// A version 5 MSFS fuel system, imported as its whole graph.
    pub fuel_graph: Option<msfs_fuel::Graph>,
    /// What the aircraft's own systems code holds, found but not converted.
    pub code: Vec<String>,
    /// The A380 for X-Plane 12 plugin's deep systems, when its source was found.
    pub deep: Option<plugin::Deep>,
}

fn num(map: &HashMap<String, String>, k: &str) -> Option<f64> {
    map.get(k).and_then(|v| v.trim().parse::<f64>().ok())
}

/// Reads an X-Plane .acf.
pub fn from_acf(path: &Path) -> Result<Spec, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut p = HashMap::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("P ") {
            if let Some((k, v)) = rest.split_once(' ') {
                p.insert(k.to_owned(), v.to_owned());
            }
        }
    }
    if p.is_empty() {
        return Err(format!("`{}` has no properties; is it an X-Plane 11/12 .acf?", path.display()));
    }
    let g = |k: &str| num(&p, k);
    let mut s = Spec { name: path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), source: path.display().to_string(), ..Default::default() };
    s.batteries = g("acf/_num_batteries").unwrap_or(1.0).max(0.0) as usize;
    s.battery_voltage = g("acf/_nom_bat_volt").unwrap_or(24.0);
    s.battery_ah = g("acf/_battery_watt_hr_max").map_or(30.0, |wh| wh / s.battery_voltage.max(1.0));
    s.generators = g("acf/_num_generators").unwrap_or(0.0).max(0.0) as usize;
    s.generator_voltage = g("acf/_nom_gen_volt").unwrap_or(28.0);
    s.generator_w = g("acf/_max_gen_amp_per_gen/0").or(g("acf/_max_gen_amp0")).unwrap_or(60.0) * s.generator_voltage;
    s.buses = g("acf/_num_buses").unwrap_or(1.0).max(1.0) as usize;
    s.inverters = g("acf/_num_inverters").unwrap_or(0.0).max(0.0) as usize;
    let n_eng = g("acf/_num_engn").unwrap_or(0.0).max(0.0) as usize;
    for i in 0..n_eng {
        let ty = p.get(&format!("_engn/{i}/_type")).cloned().unwrap_or_default();
        let kind = if ty.starts_with("RCP") {
            EngineKind::Piston
        } else if ty.contains("TURB") && !ty.starts_with("JET") {
            EngineKind::Turboprop
        } else {
            EngineKind::Jet
        };
        let thrust = g(&format!("_engn/{i}/_thrust_max_limit")).unwrap_or(0.0) * LBF;
        let power = g(&format!("_engn/{i}/_power_max_limit")).filter(|x| *x > 0.0).or(g("acf/_power_max_limit")).unwrap_or(0.0) * HP;
        let rpm = g(&format!("_prop/{i}/_des_rpm_prp")).unwrap_or(2700.0);
        let dia = g(&format!("_blad/{i}/_semilen_SEG")).or(g("_blad/0/_semilen_SEG")).map_or(1.9, |r| 2.0 * r * FT * 1.08);
        if kind == EngineKind::Jet && thrust <= 0.0 {
            s.notes.push(format!("engine {}: no thrust in the .acf; set `design_thrust` yourself", i + 1));
        }
        s.engines.push(Engine { kind, thrust_n: thrust, power_w: power, rpm, prop_diameter_m: dia });
    }
    let total = g("acf/_m_fuel_max_tot").unwrap_or(0.0) * LB;
    for i in 0..9 {
        let rat = g(&format!("acf/_tank_rat/{i}")).unwrap_or(0.0);
        if rat <= 0.0 {
            continue;
        }
        let name = p.get(&format!("acf/_tank_name/{i}")).cloned().unwrap_or_else(|| format!("Tank {}", i + 1));
        s.tanks.push(Tank { name, capacity_kg: total * rat, arm_m: g(&format!("acf/_tank_xyz/{i},2")).unwrap_or(0.0) * FT, lateral_m: g(&format!("acf/_tank_xyz/{i},0")).unwrap_or(0.0) * FT });
    }
    s.empty_kg = g("acf/_m_empty").unwrap_or(0.0) * LB;
    s.notes.push("the empty-weight arm is not in the .acf; weight.station `empty` uses 0 m, the reference point".into());
    let retract = (0..10).filter_map(|i| {
        let ty = g(&format!("_gear/{i}/_gear_type"))?;
        let t = g(&format!("_gear/{i}/_cyc_time")).unwrap_or(0.0);
        (ty >= 3.0 && t > 1.0).then_some(t)
    });
    s.retracting_gear = retract.fold(None, |a: Option<f64>, t| Some(a.map_or(t, |x| x.max(t))));
    s.hydraulic_psi = g("acf/_hyd_max_p").filter(|p| *p > 100.0);
    Ok(s)
}

type Ini = HashMap<String, HashMap<String, String>>;

fn ini(text: &str) -> Ini {
    let mut out: Ini = HashMap::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_uppercase();
        } else if let Some((k, v)) = line.split_once('=') {
            out.entry(section.clone()).or_default().insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    out
}

fn find_cfg(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    entries.sort();
    entries.into_iter().find_map(|d| find_cfg(&d, name, depth - 1))
}

fn fields(v: &str) -> HashMap<String, String> {
    v.split('#').filter_map(|f| f.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned())).collect()
}

/// Merges `over` onto `base`, section by section, key by key.
fn merge(mut base: Ini, over: Ini) -> Ini {
    for (sec, keys) in over {
        if sec == "MODULAR_MERGE" {
            continue;
        }
        base.entry(sec).or_default().extend(keys);
    }
    base
}

/// The shared `common/config/<name>` a modular preset merges onto, looking up from `from`.
fn common_cfg(from: &Path, name: &str) -> Option<PathBuf> {
    let mut at = from.parent();
    for _ in 0..6 {
        let d = at?;
        let c = d.join("common").join("config").join(name);
        if c.is_file() {
            return Some(c);
        }
        at = d.parent();
    }
    None
}

/// A cfg file, merged onto its shared base when it is a `[MODULAR_MERGE]` preset.
fn read_cfg(dir: &Path, name: &str, notes: &mut Vec<String>) -> Option<Ini> {
    let found = find_cfg(dir, name, 4);
    let own = found.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| ini(&t));
    let modular = own.as_ref().is_none_or(|i| i.contains_key("MODULAR_MERGE"));
    let base = if modular { common_cfg(found.as_deref().unwrap_or(&dir.join(name)), name).or_else(|| common_cfg(&dir.join(name), name)) } else { None };
    match (own, base.and_then(|b| std::fs::read_to_string(&b).ok().map(|t| (b, ini(&t))))) {
        (Some(o), Some((b, bi))) => {
            notes.push(format!("{name}: merged onto {}", b.display().to_string().replace('\\', "/")));
            Some(merge(bi, o))
        }
        (None, Some((b, bi))) => {
            notes.push(format!("{name}: read from {}", b.display().to_string().replace('\\', "/")));
            Some(bi)
        }
        (o, None) => o,
    }
}

/// Reads an MSFS aircraft folder's cfg files, following modular presets to their shared files.
pub fn from_msfs(dir: &Path) -> Result<Spec, String> {
    let mut merge_notes = Vec::new();
    let mut read = |n: &str| read_cfg(dir, n, &mut merge_notes);
    let fm = read("flight_model.cfg").ok_or_else(|| format!("no flight_model.cfg under `{}`", dir.display()))?;
    let en = read("engines.cfg").unwrap_or_default();
    let sy = read("systems.cfg").unwrap_or_default();
    let ac = read("aircraft.cfg").unwrap_or_default();
    let title = ac.get("FLTSIM.0").and_then(|f| f.get("title")).map(|t| t.trim_matches('"').to_owned());
    let mut s = Spec { name: title.unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), source: dir.display().to_string(), ..Default::default() };
    s.notes.append(&mut merge_notes);
    let sec = |c: &Ini, n: &str| c.get(n).cloned().unwrap_or_default();
    let wb = sec(&fm, "WEIGHT_AND_BALANCE");
    s.empty_kg = num(&wb, "empty_weight").unwrap_or(0.0) * LB;
    s.empty_arm_m = wb.get("empty_weight_cg_position").and_then(|v| v.split(',').next()).and_then(|z| z.trim().parse::<f64>().ok()).map_or(0.0, |z| -z * FT);
    let ge = sec(&en, "GENERALENGINEDATA");
    let ty = num(&ge, "engine_type").unwrap_or(1.0) as i64;
    let count = ge.keys().filter(|k| k.starts_with("engine.")).count().max(1);
    let kind = match ty {
        0 => EngineKind::Piston,
        5 => EngineKind::Turboprop,
        _ => EngineKind::Jet,
    };
    let thrust = num(&sec(&en, "TURBINEENGINEDATA"), "static_thrust").unwrap_or(0.0) * LBF;
    let piston = sec(&en, "PISTON_ENGINE");
    let power = num(&piston, "max_rated_hp").or(num(&sec(&en, "TURBOPROP_ENGINE"), "max_rated_shp")).unwrap_or(if kind == EngineKind::Piston { 180.0 } else { 1000.0 }) * HP;
    let rpm = num(&piston, "max_rated_rpm").unwrap_or(2700.0);
    let dia = num(&sec(&en, "PROPELLER"), "propeller_diameter").map_or(1.9, |d| d * FT);
    for _ in 0..count {
        s.engines.push(Engine { kind, thrust_n: thrust, power_w: power, rpm, prop_diameter_m: dia });
    }
    let density = if kind == EngineKind::Piston { 0.72 } else { 0.803 };
    s.fuel_graph = msfs_fuel::read(&sec(&fm, "FUEL_SYSTEM"), density);
    if let Some(g) = &s.fuel_graph {
        for (tid, title, kg, arm, _) in &g.tanks {
            let _ = tid;
            s.tanks.push(Tank { name: title.clone(), capacity_kg: *kg, arm_m: *arm, lateral_m: 0.0 });
        }
        s.notes.extend(g.skipped.iter().cloned());
    }
    let mut fuel = sec(&fm, "FUEL");
    fuel.extend(sec(&fm, "FUEL_SYSTEM").into_iter().filter(|(k, _)| k.starts_with("tank.")));
    let mut keys: Vec<&String> = fuel.keys().collect();
    keys.sort();
    for k in keys.into_iter().filter(|_| s.fuel_graph.is_none()) {
        let v = &fuel[k];
        if k.starts_with("tank.") {
            let f = fields(v);
            let cap = f.get("capacity").and_then(|c| c.parse::<f64>().ok()).unwrap_or(0.0);
            let pos: Vec<f64> = f.get("position").map(|p| p.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
            if cap > 0.0 {
                s.tanks.push(Tank { name: f.get("title").or(f.get("name")).cloned().unwrap_or_else(|| k.clone()), capacity_kg: cap * GAL * density, arm_m: -pos.first().copied().unwrap_or(0.0) * FT, lateral_m: pos.get(1).copied().unwrap_or(0.0) * FT });
            }
        } else {
            let parts: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if parts.len() >= 4 && parts[3] > 0.0 {
                s.tanks.push(Tank { name: k.clone(), capacity_kg: parts[3] * GAL * density, arm_m: -parts[0] * FT, lateral_m: parts[1] * FT });
            }
        }
    }
    let el = sec(&sy, "ELECTRICAL");
    let bats: Vec<&String> = el.iter().filter(|(k, _)| k.starts_with("battery.")).map(|(_, v)| v).collect();
    s.batteries = bats.len().max(1);
    s.battery_voltage = num(&el, "max_battery_voltage").unwrap_or(24.0);
    s.battery_ah = bats.first().and_then(|b| fields(b).get("capacity").and_then(|c| c.parse::<f64>().ok())).filter(|c| *c > 0.0 && *c < 1000.0).unwrap_or_else(|| {
        s.notes.push("battery capacity not found; 40 Ah assumed".into());
        40.0
    });
    let alts = el.keys().filter(|k| k.starts_with("alternator.") || k.starts_with("generator.")).count();
    s.generators = if alts > 0 { alts } else { count };
    s.generator_voltage = num(&el, "generator_alternator_voltage").unwrap_or(28.0);
    s.generator_w = num(&el, "max_generator_alternator_amps").unwrap_or(60.0) * s.generator_voltage;
    s.buses = el.keys().filter(|k| k.starts_with("bus.")).count().max(1);
    let cp = sec(&fm, "CONTACT_POINTS");
    let retract = cp.iter().filter(|(k, _)| k.starts_with("point.")).filter_map(|(_, v)| {
        let c: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (c.first() == Some(&1.0) && c.len() > 12 && c[11] > 1.0).then(|| c[11])
    });
    s.retracting_gear = retract.fold(None, |a: Option<f64>, t| Some(a.map_or(t, |x| x.max(t))));
    s.hydraulic_psi = num(&sec(&sy, "HYDRAULIC_SYSTEM"), "normal_pressure").filter(|p| *p > 100.0);
    s.code = code::scan(dir);
    if el.is_empty() {
        s.notes.push("systems.cfg has no electrical system: this aircraft runs its own systems code, listed below; the electrical, hydraulic and other systems here are a starting point".into());
    } else {
        s.notes.push("MSFS electrical circuits and logic are not converted; the files give a starting electrical system to refine".into());
    }
    Ok(s)
}

fn id(name: &str) -> String {
    let mut out: String = name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_owned();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) { format!("t_{out}") } else { out }
}

/// The files to write, path then text, and the report.
pub fn files(s: &Spec) -> (Vec<(String, String)>, String) {
    let mut files = Vec::new();
    let mut inc = vec!["systems/environment.toml", "systems/cockpit.toml", "systems/electrical.toml"];
    let head = format!("# Imported from {} by easyacft import\n", s.source.replace('\\', "/"));
    let mut env = head.clone();
    env += "[[environment.air]]\nid = \"air\"\n\n[bindings]\n";
    for (dr, path, unit) in [
        ("sim/weather/barometer_current_inhg", "air.pressure", Some("inHg")),
        ("sim/weather/temperature_ambient_c", "air.temperature", Some("degC")),
        ("sim/flightmodel/position/elevation", "air.altitude", None),
        ("sim/flightmodel/position/true_airspeed", "air.airspeed", None),
        ("sim/flightmodel/position/groundspeed", "air.groundspeed", None),
        ("sim/flightmodel/position/vh_ind", "air.vertical_speed", None),
        ("sim/flightmodel/misc/machno", "air.mach", None),
        ("sim/flightmodel/failures/onground_any", "air.on_ground", None),
        ("sim/flightmodel/failures/frm_ice", "air.icing", None),
        ("sim/flightmodel/position/alpha", "air.aoa", None),
        ("sim/flightmodel/position/psi", "air.heading", None),
        ("sim/flightmodel/position/theta", "air.pitch", None),
        ("sim/flightmodel/position/phi", "air.roll", None),
        ("sim/flightmodel/position/latitude", "air.latitude", None),
        ("sim/flightmodel/position/longitude", "air.longitude", None),
    ] {
        match unit {
            Some(u) => writeln!(env, "\"{dr}\" = {{ to = \"{path}\", unit = \"{u}\" }}").unwrap(),
            None => writeln!(env, "\"{dr}\" = {{ to = \"{path}\" }}").unwrap(),
        }
    }
    let mut ck = head.clone();
    let mut binds = String::from("\n[bindings]\n");
    for b in 1..=s.batteries {
        writeln!(ck, "[[cockpit.switch]]\nid = \"bat{b}_sw\"\n").unwrap();
        writeln!(binds, "\"sim/cockpit2/electrical/battery_on[{}]\" = {{ to = \"bat{b}_sw.on\" }}", b - 1).unwrap();
    }
    for g in 1..=s.generators {
        writeln!(ck, "[[cockpit.switch]]\nid = \"gen{g}_sw\"\ninitial = true\n").unwrap();
        writeln!(binds, "\"sim/cockpit2/electrical/generator_on[{}]\" = {{ to = \"gen{g}_sw.on\" }}", g - 1).unwrap();
    }
    for e in 1..=s.engines.len() {
        writeln!(ck, "[[cockpit.switch]]\nid = \"eng{e}_running\"\n\n[[input.number]]\nid = \"eng{e}_fuel_flow\"\n").unwrap();
        writeln!(binds, "\"sim/flightmodel/engine/ENGN_running[{}]\" = {{ to = \"eng{e}_running.on\" }}", e - 1).unwrap();
        writeln!(binds, "\"sim/cockpit2/engine/indicators/fuel_flow_kg_sec[{}]\" = {{ to = \"eng{e}_fuel_flow.value\" }}", e - 1).unwrap();
    }
    writeln!(ck, "[[cockpit.switch]]\nid = \"gear_lever\"\ninitial = true\n").unwrap();
    writeln!(binds, "\"sim/cockpit2/controls/gear_handle_down\" = {{ to = \"gear_lever.on\" }}").unwrap();
    ck += &binds;
    let mut el = head.clone();
    let v = s.generator_voltage.max(s.battery_voltage);
    for b in 1..=s.buses {
        writeln!(el, "[[electrical.bus]]\nid = \"bus{b}\"\nnominal = \"{v:.0} V\"\n").unwrap();
    }
    for b in 1..=s.batteries {
        writeln!(el, "[[electrical.bus]]\nid = \"hot_bat{b}\"\nnominal = \"{v:.0} V\"\n").unwrap();
        writeln!(el, "[[electrical.battery]]\nid = \"bat{b}\"\nbus = \"hot_bat{b}\"\nvoltage = \"{:.1} V\"\ncapacity = \"{:.1} Ah\"\n", s.battery_voltage, s.battery_ah.max(1.0)).unwrap();
        writeln!(el, "[[electrical.contactor]]\nid = \"bat_lc{b}\"\nbetween = [\"hot_bat{b}\", \"bus{}\"]\nclose_when = \"bat{b}_sw.on\"\n", (b - 1) % s.buses + 1).unwrap();
    }
    for g in 1..=s.generators {
        let eng = ((g - 1) % s.engines.len().max(1)) + 1;
        let driven = if s.engines.is_empty() { "false".to_owned() } else { format!("eng{eng}_running.on") };
        writeln!(el, "[[electrical.bus]]\nid = \"gen{g}_out\"\nnominal = \"{v:.0} V\"\n").unwrap();
        writeln!(el, "[[electrical.generator]]\nid = \"gen{g}\"\nbus = \"gen{g}_out\"\nvoltage = \"{:.1} V\"\nrating = \"{:.0} W\"\ndriven_when = \"{driven}\"\n", s.generator_voltage, s.generator_w.max(100.0)).unwrap();
        writeln!(el, "[[electrical.contactor]]\nid = \"glc{g}\"\nbetween = [\"gen{g}_out\", \"bus{}\"]\nclose_when = \"gen{g}_sw.on && gen{g}.ready\"\n", (g - 1) % s.buses + 1).unwrap();
    }
    for b in 2..=s.buses {
        writeln!(el, "[[electrical.contactor]]\nid = \"tie{b}\"\nbetween = [\"bus1\", \"bus{b}\"]\n").unwrap();
    }
    for i in 1..=s.inverters {
        writeln!(el, "[[electrical.bus]]\nid = \"ac{i}\"\nnominal = \"115 V\"\n\n[[electrical.inverter]]\nid = \"inv{i}\"\ninput = \"bus1\"\noutput = \"ac{i}\"\nvoltage = \"115 V\"\nrating = \"1000 W\"\n").unwrap();
    }
    let mut units = Vec::new();
    if let Some(d) = s.deep.as_ref().filter(|d| !d.loads.is_empty() || d.catalogue.is_some()) {
        (el, units) = plugin::electrical(d, &head, s.engines.len());
    }
    if let Some(c) = s.deep.as_ref().and_then(|d| d.catalogue.as_ref()) {
        files.extend(plugin::catalogue_files(c, &head));
        inc.extend(["systems/failures.toml", "systems/components.toml", "systems/ecam.toml"]);
    }
    files.push(("systems/environment.toml".to_owned(), env));
    files.push(("systems/cockpit.toml".to_owned(), ck));
    files.push(("systems/electrical.toml".to_owned(), el));
    if let Some(g) = &s.fuel_graph {
        let mut fuel = msfs_fuel::write(g, &head, s.engines.len(), s.deep.is_some() && s.engines.len() == 4);
        if let Some(c) = s.deep.as_ref().and_then(|d| d.catalogue.as_ref()) {
            fuel = deep_parts::fuel_failures(&fuel, c);
        }
        files.push(("systems/fuel.toml".to_owned(), fuel));
        inc.push("systems/fuel.toml");
        let mut w = head.clone();
        writeln!(w, "[[weight.station]]\nid = \"empty\"\nmass = \"{:.0} kg\"\narm = \"{:.2} m\"\n", s.empty_kg.max(1.0), s.empty_arm_m).unwrap();
        writeln!(w, "[[weight.balance]]\nid = \"wb\"\nstations = [\"empty\", {}]\n", g.tanks.iter().map(|t| format!("\"{}\"", t.0)).collect::<Vec<_>>().join(", ")).unwrap();
        files.push(("systems/weight.toml".to_owned(), w));
        inc.push("systems/weight.toml");
    } else if !s.tanks.is_empty() {
        let mut f = head.clone();
        let mut ids = Vec::new();
        for t in &s.tanks {
            let tid = id(&t.name);
            ids.push(tid.clone());
            writeln!(f, "[[fuel.tank]]\nid = \"{tid}\"\ncapacity = \"{:.0} kg\"\nquantity = \"{:.0} kg\"\narm = \"{:.2} m\"\n", t.capacity_kg, t.capacity_kg * 0.5, t.arm_m).unwrap();
        }
        for e in 1..=s.engines.len() {
            let tank = &ids[(e - 1) % ids.len()];
            let demand = if s.deep.is_some() { format!("fadec{e}.fuel_command") } else { format!("eng{e}_fuel_flow.value * 1 kg/s") };
            writeln!(f, "[[fuel.line]]\nid = \"feed{e}\"\n\n[[fuel.valve]]\nid = \"feed{e}_gravity\"\nbetween = [\"{tank}\", \"feed{e}\"]\n\n[[fuel.feed]]\nid = \"eng{e}_feed\"\nline = \"feed{e}\"\ndemand = \"{demand}\"\nminimum = \"0.01 psi\"\n").unwrap();
        }
        files.push(("systems/fuel.toml".to_owned(), f));
        inc.push("systems/fuel.toml");
        let mut w = head.clone();
        writeln!(w, "[[weight.station]]\nid = \"empty\"\nmass = \"{:.0} kg\"\narm = \"{:.2} m\"\n", s.empty_kg.max(1.0), s.empty_arm_m).unwrap();
        writeln!(w, "[[weight.balance]]\nid = \"wb\"\nstations = [\"empty\", {}]\n", ids.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", ")).unwrap();
        files.push(("systems/weight.toml".to_owned(), w));
        inc.push("systems/weight.toml");
    }
    if let Some(t) = s.retracting_gear {
        let mut g = head.clone();
        let circuit = if s.hydraulic_psi.is_some() { "circuit = \"hyd\"\n" } else { "" };
        writeln!(g, "[[gear.leg]]\nid = \"gear\"\n{circuit}down_when = \"gear_lever.on\"\ntime = \"{t:.0} s\"\n").unwrap();
        files.push(("systems/gear.toml".to_owned(), g));
        inc.push("systems/gear.toml");
    }
    if let Some(psi) = s.hydraulic_psi {
        let mut h = head.clone();
        writeln!(h, "[[hydraulic.circuit]]\nid = \"hyd\"\nnominal = \"{psi:.0} psi\"\nfluid = \"20 L\"\nlow_pressure = \"{:.0} psi\"\n", psi * 0.5).unwrap();
        for e in 1..=s.engines.len() {
            writeln!(h, "[[hydraulic.pump]]\nid = \"edp{e}\"\ncircuit = \"hyd\"\npressure = \"{psi:.0} psi\"\nflow = \"60 L/min\"\nrunning_when = \"eng{e}_running.on\"\n").unwrap();
        }
        files.push(("systems/hydraulic.toml".to_owned(), h));
        inc.push("systems/hydraulic.toml");
    }
    let mut eng = head.clone();
    for (k, e) in s.engines.iter().enumerate() {
        let n = k + 1;
        let fuel = if s.tanks.is_empty() { format!("eng{n}_fuel_flow.value * 1 kg/s") } else { format!("eng{n}_feed.flow") };
        match e.kind {
            EngineKind::Jet => writeln!(eng, "[[engine.gas_turbine]]\nid = \"eng{n}_model\"\nair = \"air\"\nfuel_flow = \"{fuel}\"\ndesign_thrust = \"{:.0} N\"\nairflow = \"{:.0} kg/s\"\nbypass_ratio = 5\nfan_pr = 1.6\nlpc_pr = 1.4\nhpc_pr = 14\n", e.thrust_n.max(1000.0), (e.thrust_n / 300.0).max(5.0)).unwrap(),
            EngineKind::Turboprop => writeln!(eng, "[[engine.gas_turbine]]\nid = \"eng{n}_model\"\nair = \"air\"\nfuel_flow = \"{fuel}\"\ndesign_power = \"{:.0} W\"\nairflow = \"{:.1} kg/s\"\nspools = 2\nhpc_pr = 10\noutput_rpm = \"prop{n}.rpm\"\n\n[[engine.propeller]]\nid = \"prop{n}\"\nair = \"air\"\npower = \"eng{n}_model.shaft_power\"\ndiameter = \"{:.2} m\"\nrated_rpm = \"{:.0} rpm\"\nrated_power = \"{:.0} W\"\n", e.power_w.max(1000.0), (e.power_w / 250_000.0).max(1.0), e.prop_diameter_m, e.rpm, e.power_w.max(1000.0)).unwrap(),
            EngineKind::Piston => writeln!(eng, "[[engine.piston]]\nid = \"eng{n}_model\"\nair = \"air\"\nrated_power = \"{:.0} W\"\nrated_rpm = \"{:.0} rpm\"\nrpm = \"prop{n}.rpm\"\n\n[[engine.propeller]]\nid = \"prop{n}\"\nair = \"air\"\npower = \"eng{n}_model.power\"\ndiameter = \"{:.2} m\"\nrated_rpm = \"{:.0} rpm\"\nrated_power = \"{:.0} W\"\n", e.power_w.max(1000.0), e.rpm, e.prop_diameter_m, e.rpm, e.power_w.max(1000.0)).unwrap(),
        }
    }
    if !s.engines.is_empty() {
        files.push(("systems/engines.toml".to_owned(), eng));
    }
    if let Some(d) = s.deep.as_ref().filter(|_| s.engines.len() == 4) {
        let feeds: Vec<Option<String>> = (1..=4).map(|n| match &s.fuel_graph { Some(g) => g.feeds.iter().find(|f| f.2 == Some(n)).map(|f| f.0.clone()), None => (!s.tanks.is_empty()).then(|| format!("eng{n}_feed")) }).collect();
        let mut made = plugin::systems(d, &head, &units, &feeds);
        if s.retracting_gear.is_none() {
            made[1].1 += "[[cockpit.switch]]\nid = \"gear_lever\"\ninitial = true\n";
        }
        files.retain(|(p, _)| !made.iter().any(|m| &m.0 == p) && p != "systems/gear.toml");
        inc.retain(|i| *i != "systems/gear.toml" && *i != "systems/hydraulic.toml");
        files.extend(made);
        inc.extend(["systems/engines.toml", "systems/hydraulic.toml", "systems/computers.toml"]);
        for (path, text) in deep_parts::files(d, &head, &units) {
            inc.push(match path.as_str() {
                "systems/avionics.toml" => "systems/avionics.toml",
                "systems/protection.toml" => "systems/protection.toml",
                "systems/oxygen.toml" => "systems/oxygen.toml",
                "systems/pneumatic.toml" => "systems/pneumatic.toml",
                "systems/brakes.toml" => "systems/brakes.toml",
                _ => "systems/lights.toml",
            });
            files.push((path, text));
        }
        let at = Path::new(&d.root);
        let tree = [at.join("fbw").join("xplane"), at.join("..").join("fbw-aircraft")].into_iter().find(|t| t.join("fbw-a380x").is_dir());
        if let Some(t) = tree {
            let mut net = afdx::network(&t, &units, &head).unwrap_or_else(|| head.clone());
            let has: std::collections::HashSet<String> = files.iter().flat_map(|(_, text)| text.lines().filter_map(|l| l.strip_prefix("id = \"").and_then(|r| r.strip_suffix('"')).map(str::to_owned)).collect::<Vec<_>>()).collect();
            net += &fbw_links::links(&t, &has);
            if net.len() > head.len() {
                files.push(("systems/network.toml".to_owned(), net));
                inc.push("systems/network.toml");
            }
        }
    }
    let mut root = format!("{head}[aircraft]\nname = \"{}\"\neasyacft = \"1.0\"\ninclude = [{}]\n", s.name.replace('"', "'"), inc.iter().map(|i| format!("\"{i}\"")).collect::<Vec<_>>().join(", "));
    root += "\n[rates]\nelectrical = \"10 ms\"\n";
    files.insert(0, ("aircraft.toml".to_owned(), root));
    let mut rep = format!("# Import report: {}\n\nSource: {}\n\n## Found\n\n", s.name, s.source.replace('\\', "/"));
    writeln!(rep, "- {} batteries, {:.0} V, {:.0} Ah each", s.batteries, s.battery_voltage, s.battery_ah).unwrap();
    writeln!(rep, "- {} generators, {:.0} V, {:.0} W each", s.generators, s.generator_voltage, s.generator_w).unwrap();
    writeln!(rep, "- {} buses, {} inverters", s.buses, s.inverters).unwrap();
    for (k, e) in s.engines.iter().enumerate() {
        writeln!(rep, "- engine {}: {:?}, {:.0} N, {:.0} W", k + 1, e.kind, e.thrust_n, e.power_w).unwrap();
    }
    for t in &s.tanks {
        writeln!(rep, "- tank {}: {:.0} kg at {:.2} m", t.name, t.capacity_kg, t.arm_m).unwrap();
    }
    writeln!(rep, "- empty mass {:.0} kg", s.empty_kg).unwrap();
    if let Some(t) = s.retracting_gear {
        writeln!(rep, "- retracting gear, {t:.0} s").unwrap();
    }
    if let Some(p) = s.hydraulic_psi {
        writeln!(rep, "- hydraulics at {p:.0} psi").unwrap();
    }
    if let Some(g) = &s.fuel_graph {
        writeln!(rep, "- fuel system graph: {} tanks, {} pumps, {} valves, {} pipes and {} junction, pump and valve nodes from {} lines, {} engine and APU feeds", g.tanks.len(), g.pumps.len(), g.valves.len(), g.pipes.len(), g.nodes.len(), g.lines, g.feeds.len()).unwrap();
    }
    if !s.code.is_empty() {
        rep += "\n## In the aircraft's systems code\n\nFound by reading its source; the behaviour is compiled code and is not converted.\n\n";
        for c in &s.code {
            writeln!(rep, "- {c}").unwrap();
        }
    }
    if let Some(d) = &s.deep {
        rep += &plugin::report(d);
        files.push(("DEEP.md".to_owned(), plugin::appendix(d)));
    }
    rep += "\n## To do\n\n";
    if s.deep.is_some() && s.engines.len() == 4 {
        rep += "- Engines, FADECs, APU, hydraulics, gear and computers come from the plugin; X-Plane still makes the thrust until `output.force` parts are added.\n";
    } else {
        rep += "- X-Plane still flies the engines. `systems/engines.toml` has EasyAcft models for them; add it to `include`, wire each FADEC or throttle, and add `output.force` parts to let them make the thrust.\n";
    }
    rep += "- Bus layout, ties and loads are a starting point; compare with the aircraft's documentation.\n";
    for n in &s.notes {
        writeln!(rep, "- {}", n[..1].to_uppercase() + &n[1..]).unwrap();
    }
    (files, rep)
}

/// Imports `source` (a .acf file or an MSFS aircraft folder) into `out`.
pub fn import(source: &Path, out: &Path) -> Result<String, String> {
    import_with(source, out, None, None)
}

/// Imports `source` with the plugin source at `code` (or found above `source`) and its catalogue, exported when not given.
pub fn import_with(source: &Path, out: &Path, code: Option<&Path>, catalogue: Option<&Path>) -> Result<String, String> {
    let mut spec = if source.extension().is_some_and(|e| e.eq_ignore_ascii_case("acf")) { from_acf(source)? } else { from_msfs(source)? };
    let root = code.and_then(plugin::find).or_else(|| plugin::find(source));
    if let Some(r) = root {
        let mut d = plugin::scan(&r);
        let json = match catalogue {
            Some(c) => c.to_path_buf(),
            None => {
                std::fs::create_dir_all(out).map_err(|e| format!("cannot create `{}`: {e}", out.display()))?;
                let j = std::path::absolute(out.join("plugin-catalogue.json")).unwrap_or_else(|_| out.join("plugin-catalogue.json"));
                plugin::export_catalogue(&r, &j)?;
                j
            }
        };
        let text = std::fs::read_to_string(&json).map_err(|e| format!("cannot read `{}`: {e}", json.display()))?;
        d.catalogue = Some(catalogue::read(&text).map_err(|e| format!("`{}`: {e}", json.display()))?);
        spec.deep = Some(d);
        // The plugin compiles in FlyByWire's flight_model.cfg: its [FUEL_SYSTEM] is the aircraft's whole fuel graph.
        let tree = [r.join("fbw").join("xplane"), r.join("..").join("fbw-aircraft")].into_iter().find(|t| t.join("fbw-a380x").is_dir());
        let cfg = tree.map(|t| t.join("fbw-a380x/src/base/flybywire-aircraft-a380-842/SimObjects/AirPlanes/FlyByWire_A380X/common/config/flight_model.cfg"));
        if let Some(fm) = cfg.and_then(|p| std::fs::read_to_string(p).ok()).map(|t| ini(&t)) {
            if let Some(g) = msfs_fuel::read(&fm.get("FUEL_SYSTEM").cloned().unwrap_or_default(), 0.803) {
                spec.tanks = g.tanks.iter().map(|(_, title, kg, arm, _)| Tank { name: title.clone(), capacity_kg: *kg, arm_m: *arm, lateral_m: 0.0 }).collect();
                spec.notes.push(format!("fuel: FlyByWire's [FUEL_SYSTEM] graph the plugin builds in, {} tanks, {} pumps, {} valves", g.tanks.len(), g.pumps.len(), g.valves.len()));
                spec.fuel_graph = Some(g);
            }
        }
    } else if let Some(c) = code {
        return Err(format!("no `src/deep` under `{}`", c.display()));
    }
    let (files, report) = files(&spec);
    for (rel, text) in &files {
        let p = out.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("cannot create `{}`: {e}", d.display()))?;
        }
        std::fs::write(&p, text).map_err(|e| format!("cannot write `{}`: {e}", p.display()))?;
    }
    std::fs::write(out.join("IMPORT.md"), &report).map_err(|e| format!("cannot write the report: {e}"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_acf_becomes_a_spec() {
        let dir = std::env::temp_dir().join(format!("easyacft-imp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let acf = dir.join("Twin.acf");
        std::fs::write(&acf, "I\n1200 Version\nACF\nPROPERTIES_BEGIN\nP acf/_num_batteries 2\nP acf/_nom_bat_volt 24\nP acf/_battery_watt_hr_max 960\nP acf/_num_generators 2\nP acf/_nom_gen_volt 28\nP acf/_max_gen_amp_per_gen/0 150\nP acf/_num_buses 2\nP acf/_num_engn 2\nP _engn/0/_type JET_2SPOOL\nP _engn/1/_type JET_2SPOOL\nP _engn/0/_thrust_max_limit 27000\nP _engn/1/_thrust_max_limit 27000\nP acf/_m_fuel_max_tot 46000\nP acf/_tank_rat/0 0.2\nP acf/_tank_rat/1 0.6\nP acf/_tank_rat/2 0.2\nP acf/_tank_name/0 Left\nP acf/_tank_name/1 Center\nP acf/_tank_name/2 Right\nP acf/_m_empty 91000\nP _gear/0/_gear_type 3\nP _gear/0/_cyc_time 9\nP acf/_hyd_max_p 3000\nPROPERTIES_END\n").unwrap();
        let s = from_acf(&acf).unwrap();
        assert_eq!((s.batteries, s.generators, s.buses), (2, 2, 2));
        assert!((s.battery_ah - 40.0).abs() < 1e-9);
        assert_eq!(s.engines.len(), 2);
        assert_eq!(s.engines[0].kind, EngineKind::Jet);
        assert!((s.engines[0].thrust_n - 27000.0 * LBF).abs() < 1.0);
        assert_eq!(s.tanks.len(), 3);
        assert!((s.tanks[1].capacity_kg - 46000.0 * 0.6 * LB).abs() < 1.0);
        assert_eq!(s.retracting_gear, Some(9.0));
        assert_eq!(s.hydraulic_psi, Some(3000.0));
        let out = dir.join("out");
        let report = import(&acf, &out).unwrap();
        assert!(report.contains("2 batteries"));
        assert!(out.join("systems/fuel.toml").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn msfs_ini_and_tanks_parse() {
        let dir = std::env::temp_dir().join(format!("easyacft-msfs-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("SimObjects/Airplanes/Small")).unwrap();
        let d = dir.join("SimObjects/Airplanes/Small");
        std::fs::write(d.join("flight_model.cfg"), "[WEIGHT_AND_BALANCE]\nempty_weight = 1700 ; lb\nempty_weight_CG_position = -2, 0, 0\n[FUEL]\nLeftMain = -1, -5, 2, 26, 1\nRightMain = -1, 5, 2, 26, 1\n[CONTACT_POINTS]\npoint.0 = 1, 3, 0, -4, 1500, 0, 0.5, 30, 0.2, 2.5, 0.7, 0, 0, 0, 0, 0\n").unwrap();
        std::fs::write(d.join("engines.cfg"), "[GENERALENGINEDATA]\nengine_type = 0\nEngine.0 = 4, 0, 0\n[PISTON_ENGINE]\nmax_rated_rpm = 2700\nmax_rated_hp = 180\n[PROPELLER]\npropeller_diameter = 6.25\n").unwrap();
        std::fs::write(d.join("systems.cfg"), "[ELECTRICAL]\nmax_battery_voltage = 24\ngenerator_alternator_voltage = 28\nmax_generator_alternator_amps = 60\n").unwrap();
        let s = from_msfs(&dir).unwrap();
        assert_eq!(s.engines.len(), 1);
        assert_eq!(s.engines[0].kind, EngineKind::Piston);
        assert!((s.engines[0].power_w - 180.0 * HP).abs() < 1.0);
        assert_eq!(s.tanks.len(), 2);
        assert!((s.tanks[0].capacity_kg - 26.0 * GAL * 0.72).abs() < 0.1);
        assert!((s.empty_arm_m - 2.0 * FT).abs() < 1e-9);
        assert_eq!(s.retracting_gear, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
