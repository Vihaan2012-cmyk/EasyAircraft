//! The plugin's catalogued units as typed parts: sensors and air data, fire, ice, oxygen, bleed air, brakes, tyres, steering and lights.

use super::catalogue::{Catalogue, Comp};
use super::plugin::{Deep, Unit};
use std::fmt::Write;

fn comp<'a>(c: &'a Catalogue, id: &str) -> Option<&'a Comp> {
    c.components.iter().find(|x| x.id == id)
}

/// The failures of `id` whose names contain any of `words` (all of them when `words` is empty), as an expression.
pub(crate) fn fails(c: &Catalogue, id: &str, words: &[&str]) -> String {
    let Some(x) = comp(c, id) else { return "false".into() };
    let names: std::collections::HashMap<u64, String> = c.failures.iter().map(|f| (f.0, f.1.to_ascii_lowercase())).collect();
    let ids: Vec<String> = x
        .failures
        .iter()
        .filter(|f| words.is_empty() || names.get(f).is_some_and(|n| words.iter().any(|w| n.contains(w))))
        .map(|f| format!("f{f}.value > 0"))
        .collect();
    if ids.is_empty() {
        "false".into()
    } else {
        ids.join(" || ")
    }
}

pub(crate) fn not(e: &str) -> String {
    if e == "false" {
        "true".into()
    } else {
        format!("!({e})")
    }
}

pub(crate) fn and_not(base: &str, e: &str) -> String {
    if e == "false" {
        base.to_owned()
    } else {
        format!("{base} && !({e})")
    }
}

fn switch(f: &mut String, id: &str, on: bool) {
    writeln!(f, "[[cockpit.switch]]\nid = \"{id}\"{}\n", if on { "\ninitial = true" } else { "" }).unwrap();
}

/// A number before `unit` in a catalogue name, such as 1850 in "(2 x 3260 L free air, 1850 psig)".
fn number_before(name: &str, unit: &str) -> Option<f64> {
    let i = name.find(unit)?;
    name[..i].trim_end().rsplit(|c: char| !(c.is_ascii_digit() || c == '.')).next()?.parse().ok()
}

fn avionics(c: &Catalogue, head: &str, units: &[Unit]) -> String {
    let has = |id: &str| units.iter().any(|u| u.0 == id);
    let mut f = head.to_owned();
    f += "# Probes and air data from the plugin's navigation sensors (34_nav), heated through the ice protection's probe heaters (30_ice).\n\n";
    switch(&mut f, "probe_heat", false);
    for n in 1..=4 {
        let id = format!("34_nav.pitot_{n}");
        if comp(c, &id).is_none() {
            continue;
        }
        let heater = fails(c, &format!("30_ice.pitot{n}_heater"), &[]);
        let broken = fails(c, &id, &["heater"]);
        writeln!(f, "[[sensor.pitot]]\nid = \"pitot{n}\"\nair = \"air\"\nheat_when = \"{}\"\nblocked_when = \"{}\"\n", and_not(&and_not("probe_heat.on", &heater), &broken), fails(c, &id, &["blockage", "drain", "damage"])).unwrap();
    }
    for n in 1..=4 {
        for side in 1..=2 {
            let id = format!("34_nav.static_{n}_{side}");
            if comp(c, &id).is_some() {
                writeln!(f, "[[sensor.static_port]]\nid = \"static{n}_{side}\"\nair = \"air\"\nblocked_when = \"{}\"\n", fails(c, &id, &["blocked"])).unwrap();
            }
        }
    }
    for n in 1..=3 {
        let id = format!("34_nav.tat_{n}");
        if comp(c, &id).is_some() {
            writeln!(f, "[[sensor.tat]]\nid = \"tat{n}\"\nair = \"air\"\nheat_when = \"{}\"\n", and_not("probe_heat.on", &fails(c, &id, &["heater"]))).unwrap();
        }
        let id = format!("34_nav.aoa_{n}");
        if comp(c, &id).is_some() {
            writeln!(f, "[[sensor.aoa]]\nid = \"aoa{n}\"\nair = \"air\"\nheat_when = \"{}\"\n", and_not("probe_heat.on", &fails(c, &format!("30_ice.aoa{n}_heater"), &[]))).unwrap();
        }
    }
    for n in 1..=3 {
        let computer = if has(&format!("adirs_{n}")) { format!("computer = \"adirs_{n}\"\n") } else { String::new() };
        let tat = if comp(c, &format!("34_nav.tat_{n}")).is_some() { format!("tat_probe = \"tat{n}\"\n") } else { String::new() };
        writeln!(f, "[[computer.air_data]]\nid = \"adr{n}\"\npitot = \"pitot{n}\"\nstatic = \"static{n}_1\"\nair = \"air\"\n{computer}{tat}vmo = \"340 kt\"\nmmo = 0.89\n").unwrap();
        writeln!(f, "[[computer.inertial]]\nid = \"ir{n}\"\nair = \"air\"\n{computer}").unwrap();
    }
    f
}

fn protection(c: &Catalogue, head: &str) -> String {
    let mut f = head.to_owned();
    f += "# Fire zones and bottles from the plugin's fire protection (26_fire); ice from its ice protection (30_ice).\n\n";
    let zones: Vec<String> = c.components.iter().filter_map(|x| x.id.strip_prefix("26_fire.")?.strip_suffix("_loop_a").map(str::to_owned)).collect();
    for z in &zones {
        let leak = fails(c, &format!("26_fire.{z}_leak_source"), &[]);
        let fire = if leak == "false" { format!("sim_{z}_fire.on") } else { format!("sim_{z}_fire.on || {leak}") };
        switch(&mut f, &format!("sim_{z}_fire"), false);
        writeln!(f, "[[fire.zone]]\nid = \"{z}_fire\"\nfire_when = \"{fire}\"\nloop_a_failed = \"{}\"\nloop_b_failed = \"{}\"\n", fails(c, &format!("26_fire.{z}_loop_a"), &[]), fails(c, &format!("26_fire.{z}_loop_b"), &[])).unwrap();
    }
    for x in c.components.iter().filter(|x| x.id.starts_with("26_fire.") && x.id.contains("bottle")) {
        let id = &x.id["26_fire.".len()..];
        let Some(zone) = zones.iter().find(|z| id.starts_with(z.as_str())) else { continue };
        switch(&mut f, &format!("{id}_pb"), false);
        writeln!(f, "# {}\n[[fire.bottle]]\nid = \"{id}\"\nzones = [\"{zone}_fire\"]\ndischarge_when = \"{id}_pb.on\"\nsquib_powered = \"{}\"\n", x.name, not(&fails(c, &x.id, &["squib"]))).unwrap();
    }
    switch(&mut f, "wing_ai", false);
    switch(&mut f, "windshield_heat", true);
    for x in c.components.iter().filter(|x| x.id.starts_with("30_ice.")) {
        let id = &x.id["30_ice.".len()..];
        let heated = if let Some(side) = id.strip_prefix("wing_").and_then(|r| r.strip_suffix("_anti_ice_valve")) {
            let _ = side;
            and_not("wing_ai.on", &fails(c, &x.id, &["stuck closed", "leak"]))
        } else if let Some(n) = id.strip_prefix("nacelle").and_then(|r| r.strip_suffix("_anti_ice_valve")) {
            switch(&mut f, &format!("eng{n}_ai"), false);
            and_not(&format!("eng{n}_ai.on && eng{n}.running"), &fails(c, &x.id, &["stuck closed", "leak"]))
        } else if id.ends_with("windshield_heat_film") {
            and_not("windshield_heat.on", &fails(c, &x.id, &[]))
        } else {
            continue;
        };
        writeln!(f, "# {}\n[[ice.surface]]\nid = \"ice_{id}\"\nair = \"air\"\nheated_when = \"{heated}\"\n", x.name).unwrap();
    }
    f
}

fn oxygen(c: &Catalogue, head: &str) -> String {
    let mut f = head.to_owned();
    f += "# Oxygen from the plugin's oxygen system (35_oxy); pressures and volumes read from its unit names.\n\n";
    switch(&mut f, "crew_o2_supply", true);
    f += "[[input.number]]\nid = \"crew_masks_on\"\n\n";
    if let Some(x) = comp(c, "35_oxy.crew_cylinder") {
        let psi = number_before(&x.name, "psig").unwrap_or(1850.0);
        let litres = number_before(&x.name, "L free air").unwrap_or(3260.0) * if x.name.contains("2 x") { 2.0 } else { 1.0 };
        let supply = and_not("crew_o2_supply.on", &fails(c, "35_oxy.crew_supply_valve", &[]));
        writeln!(f, "# {}\n[[oxygen.crew]]\nid = \"crew_o2\"\npressure = \"{psi:.0} psi\"\ncapacity = \"{litres:.0} L\"\nmasks_on = \"crew_masks_on.value\"\nsupply_when = \"{supply}\"\n", x.name).unwrap();
    }
    if let Some(x) = comp(c, "35_oxy.therapeutic_cylinder") {
        let psi = number_before(&x.name, "psig").unwrap_or(1800.0);
        let litres = number_before(&x.name, "L free air").unwrap_or(2772.0);
        writeln!(f, "# {}\n[[oxygen.crew]]\nid = \"first_aid_o2\"\npressure = \"{psi:.0} psi\"\ncapacity = \"{litres:.0} L\"\n", x.name).unwrap();
    }
    if comp(c, "35_oxy.pax_generators").is_some() {
        switch(&mut f, "pax_o2_deploy", false);
        let deploy = and_not("pax_o2_deploy.on", &fails(c, "35_oxy.pax_psu_latches", &[]));
        writeln!(f, "[[oxygen.passenger]]\nid = \"pax_o2\"\ndeploy_when = \"{deploy}\"\n").unwrap();
    }
    f
}

fn pneumatic(c: &Catalogue, head: &str) -> String {
    let mut f = head.to_owned();
    f += "# Bleed air from the plugin's pneumatic ducts (36_pneu): each engine's IP and HP ports, its pressure-regulating valve and precooler, the crossbleed manifold and the APU.\n\n";
    let duct_leak = fails(c, "36_pneu.engine_bleed_duct", &["leak", "rupture"]);
    let prv_stuck = fails(c, "36_pneu.engine_upstream_valve_stage", &["pr (shutoff) valve stuck"]);
    let hp_stuck = fails(c, "36_pneu.engine_upstream_valve_stage", &["hp valve stuck"]);
    let fan_stuck = fails(c, "36_pneu.engine_precooler", &["fan air valve stuck"]);
    let pc_check = fails(c, "36_pneu.engine_precooler", &["check valve"]);
    let ip_check = fails(c, "36_pneu.engine_upstream_valve_stage", &["ip8 check valve"]);
    let start_leak = fails(c, "36_pneu.engine_start_duct", &["leak", "rupture"]);
    let start_check = fails(c, "36_pneu.engine_start_duct", &["check valve"]);
    let opt = |k: &str, e: &str| if e == "false" { String::new() } else { format!("{k} = \"{e}\"\n") };
    for n in 1..=4 {
        let side = if n <= 2 { "manifold_l" } else { "manifold_r" };
        switch(&mut f, &format!("bleed{n}_pb"), true);
        writeln!(f, "[[pneumatic.duct]]\nid = \"bleed{n}\"\nleak_when = \"{duct_leak}\"\n").unwrap();
        writeln!(f, "[[pneumatic.source]]\nid = \"bleed{n}_ip\"\nduct = \"bleed{n}\"\npressure = \"90 psi\"\ntemperature = \"300 degC\"\nflow = \"if(eng{n}.n1 > 45, 5 kg/s, 2 kg/s)\"\navailable_when = \"{}\"\n{}", and_not(&format!("eng{n}.n3 > 30"), &ip_check), opt("check_failed_when", &pc_check)).unwrap();
        writeln!(f, "[[pneumatic.source]]\nid = \"bleed{n}_hp\"\nduct = \"bleed{n}\"\npressure = \"120 psi\"\ntemperature = \"480 degC\"\nflow = \"if(eng{n}.n1 < 45, 4 kg/s, 0 kg/s)\"\navailable_when = \"{}\"\n{}", and_not(&format!("eng{n}.n3 > 30 && eng{n}.n1 < 45"), &hp_stuck), opt("check_failed_when", &pc_check)).unwrap();
        let cool = "200 degC";
        writeln!(f, "[[pneumatic.valve]]\nid = \"prv{n}\"\nbetween = [\"bleed{n}\", \"{side}\"]\nopen_when = \"bleed{n}_pb.on && !fadec{n}.starter\"\nregulate = \"40 psi\"\ncool_to = \"{cool}\"\nstuck_when = \"{prv_stuck}\"\n{}", opt("fan_air_when", &not(&fan_stuck)).replace("fan_air_when = \"true\"\n", "")).unwrap();
    }
    for n in 1..=4 {
        let side = if n <= 2 { "manifold_l" } else { "manifold_r" };
        writeln!(f, "# Engine {n}'s air turbine starter, fed from the manifold through its start valve.\n[[pneumatic.duct]]\nid = \"start{n}_duct\"\nvolume = \"0.02 m3\"\n{}\n[[pneumatic.valve]]\nid = \"start{n}_valve\"\nbetween = [\"{side}\", \"start{n}_duct\"]\nopen_when = \"fadec{n}.starter\"\nflow = \"3 kg/s\"\n{}\n[[pneumatic.consumer]]\nid = \"starter{n}\"\nduct = \"start{n}_duct\"\nflow = \"1.2 kg/s\"\non_when = \"fadec{n}.starter\"\nturbine = true\n", opt("leak_when", &start_leak), opt("leak_when", &start_check)).unwrap();
    }
    for (id, vol) in [("manifold_l", "0.3 m3"), ("manifold_r", "0.3 m3")] {
        writeln!(f, "[[pneumatic.duct]]\nid = \"{id}\"\nvolume = \"{vol}\"\n").unwrap();
    }
    switch(&mut f, "xbleed", false);
    f += "[[pneumatic.valve]]\nid = \"xbleed_valve\"\nbetween = [\"manifold_l\", \"manifold_r\"]\nopen_when = \"xbleed.on || apu_bleed_valve.open\"\nflow = \"6 kg/s\"\n\n";
    switch(&mut f, "apu_bleed_pb", true);
    writeln!(f, "[[pneumatic.duct]]\nid = \"apu_duct\"\nleak_when = \"{}\"\n", fails(c, "36_pneu.apu_bleed_duct", &["leak", "rupture"])).unwrap();
    let apu_pc = |w: &str| fails(c, "36_pneu.apu_precooler", &[w]);
    writeln!(f, "[[pneumatic.source]]\nid = \"apu_load_compressor\"\nduct = \"apu_duct\"\npressure = \"55 psi\"\ntemperature = \"220 degC\"\nflow = \"2.4 kg/s\"\navailable_when = \"apu.available\"\n{}", opt("check_failed_when", &apu_pc("check valve"))).unwrap();
    writeln!(f, "[[pneumatic.valve]]\nid = \"apu_bleed_valve\"\nbetween = [\"apu_duct\", \"manifold_l\"]\nopen_when = \"apu_bleed_pb.on && apu.available\"\ncool_to = \"{}\"\n{}", "180 degC", opt("fan_air_when", &not(&apu_pc("fan air valve stuck"))).replace("fan_air_when = \"true\"\n", "")).unwrap();
    let pack_leak = fails(c, "36_pneu.pack_supply_duct", &["leak", "rupture"]);
    for (n, side) in [(1, "manifold_l"), (2, "manifold_r")] {
        switch(&mut f, &format!("pack{n}_pb"), true);
        writeln!(f, "[[pneumatic.duct]]\nid = \"pack{n}_supply\"\nleak_when = \"{pack_leak}\"\n\n[[pneumatic.valve]]\nid = \"pack{n}_fcv\"\nbetween = [\"{side}\", \"pack{n}_supply\"]\nopen_when = \"pack{n}_pb.on\"\n\n[[pneumatic.consumer]]\nid = \"pack{n}\"\nduct = \"pack{n}_supply\"\nflow = \"1.2 kg/s\"\non_when = \"pack{n}_pb.on\"\nregulated = true\n").unwrap();
    }
    for (side, m) in [("l", "manifold_l"), ("r", "manifold_r")] {
        let stuck = fails(c, &format!("30_ice.wing_{side}_anti_ice_valve"), &["stuck closed"]);
        writeln!(f, "[[pneumatic.consumer]]\nid = \"wing_ai_{side}\"\nduct = \"{m}\"\nflow = \"0.9 kg/s\"\non_when = \"{}\"\n", and_not("wing_ai.on", &stuck)).unwrap();
    }
    f
}

/// Wheel positions as the catalogue names them: (catalogue key, part id, leg).
fn wheels(c: &Catalogue) -> Vec<(String, String, &'static str)> {
    let mut out = Vec::new();
    for x in &c.components {
        let Some(rest) = x.id.strip_prefix("32_gear.tyre_pressure_") else { continue };
        let leg = if rest.starts_with("nose") { "nose_gear" } else if rest.contains("wing") { if rest.starts_with("left") { "wing_gear_l" } else { "wing_gear_r" } } else if rest.starts_with("left") { "body_gear_l" } else { "body_gear_r" };
        out.push((rest.to_owned(), format!("tyre_{rest}"), leg));
    }
    out
}

fn gear(c: &Catalogue, head: &str) -> String {
    let mut f = head.to_owned();
    f += "# The tyres the plugin's landing gear monitors (32_gear); the brakes and steering are in hydraulic.toml, on their circuits.\n\n";
    for (key, id, _) in wheels(c) {
        writeln!(f, "[[gear.tyre]]\nid = \"{id}\"\npressure = \"{} psi\"\nair = \"air\"\n", if key.starts_with("nose") { 180 } else { 218 }).unwrap();
    }
    f
}

fn lights(units: &[Unit], head: &str) -> String {
    let mut f = head.to_owned();
    f += "# Every lighting consumer (ATA 33) in the plugin's load catalogue, on its own breaker bus, behind one switch each.\n\n";
    for (lid, lb, _, ata, name) in units.iter().filter(|u| u.3 == 33) {
        let _ = ata;
        writeln!(f, "# {name}\n[[cockpit.switch]]\nid = \"{lid}_sw\"\n\n[[light.unit]]\nid = \"{lid}_light\"\nbus = \"{lb}\"\non_when = \"{lid}_sw.on\"\n").unwrap();
    }
    f
}

/// The typed systems files, or none without a catalogue.
pub fn files(d: &Deep, head: &str, units: &[Unit]) -> Vec<(String, String)> {
    let Some(c) = d.catalogue.as_ref() else { return Vec::new() };
    vec![
        ("systems/avionics.toml".into(), avionics(c, head, units)),
        ("systems/protection.toml".into(), protection(c, head)),
        ("systems/oxygen.toml".into(), oxygen(c, head)),
        ("systems/pneumatic.toml".into(), pneumatic(c, head)),
        ("systems/brakes.toml".into(), gear(c, head)),
        ("systems/lights.toml".into(), lights(units, head)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_come_out_of_unit_names() {
        let n = "Crew oxygen cylinder group (2 x 3260 L free air, 1850 psig)";
        assert_eq!((number_before(n, "psig"), number_before(n, "L free air")), (Some(1850.0), Some(3260.0)));
    }
}

/// The plugin's fuel failures on FlyByWire's fuel graph parts, which carry FlyByWire's names: pumps, valves,
/// tank skins and probes, and the transfer galleries.
pub fn fuel_failures(text: &str, c: &Catalogue) -> String {
    let mut add: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut put = |id: String, field: &str, e: String| {
        if e != "false" {
            add.entry(id).or_default().push(format!("{field} = \"{e}\""));
        }
    };
    let tank = |k: &str| k.replace("feed_", "feed");
    for x in c.components.iter().filter(|x| x.id.starts_with("28_fuel.")) {
        let key = &x.id["28_fuel.".len()..];
        let e = fails(c, &x.id, &[]);
        if key.starts_with("pump.") {
            put(super::msfs_fuel::id(&x.name), "impeller_damaged_when", e);
        } else if key.starts_with("valve.") {
            for n in x.name.split('/') {
                put(super::msfs_fuel::id(n), "stuck_when", e.clone());
            }
        } else if let Some(t) = key.strip_prefix("tank_wall.") {
            put(tank(t), "leak_when", e);
        } else if let Some(t) = key.strip_prefix("fqms_probes.") {
            put(tank(t), "probe_failed_when", fails(c, &x.id, &["probe failure"]));
        } else if let Some(g) = key.strip_prefix("gallery.") {
            let lines: &[&str] = if g == "aft" { &["aft_gallery_junction1", "aft_gallery_junction2"] } else { &["fwd_gallery_junction"] };
            for l in lines {
                put(l.to_string(), "leak_when", e.clone());
            }
        }
    }
    let mut out = String::new();
    for line in text.lines() {
        out += line;
        out.push('\n');
        if let Some(id) = line.strip_prefix("id = \"").and_then(|r| r.strip_suffix('"')) {
            for f in add.get(id).into_iter().flatten() {
                out += f;
                out.push('\n');
            }
        }
    }
    out
}
