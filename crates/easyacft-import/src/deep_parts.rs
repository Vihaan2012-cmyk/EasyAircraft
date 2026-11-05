//! The plugin's catalogued units as typed parts: sensors and air data, fire, ice, oxygen, bleed air, brakes, tyres, steering and lights.

use super::catalogue::{Catalogue, Comp};
use super::plugin::{Deep, Unit};
use std::fmt::Write;

fn comp<'a>(c: &'a Catalogue, id: &str) -> Option<&'a Comp> {
    c.components.iter().find(|x| x.id == id)
}

/// The failures of `id` whose names contain any of `words` (all of them when `words` is empty), as an expression.
fn fails(c: &Catalogue, id: &str, words: &[&str]) -> String {
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

fn not(e: &str) -> String {
    if e == "false" {
        "true".into()
    } else {
        format!("!({e})")
    }
}

fn and_not(base: &str, e: &str) -> String {
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
    let fouled = fails(c, "36_pneu.engine_precooler", &["fouling"]);
    for n in 1..=4 {
        let side = if n <= 2 { "manifold_l" } else { "manifold_r" };
        switch(&mut f, &format!("bleed{n}_pb"), true);
        writeln!(f, "[[pneumatic.duct]]\nid = \"bleed{n}\"\nleak_when = \"{duct_leak}\"\n").unwrap();
        writeln!(f, "[[pneumatic.source]]\nid = \"bleed{n}_ip\"\nduct = \"bleed{n}\"\npressure = \"90 psi\"\ntemperature = \"300 degC\"\nflow = \"if(eng{n}.n1 > 45, 5 kg/s, 2 kg/s)\"\navailable_when = \"eng{n}.n3 > 30\"\n").unwrap();
        writeln!(f, "[[pneumatic.source]]\nid = \"bleed{n}_hp\"\nduct = \"bleed{n}\"\npressure = \"120 psi\"\ntemperature = \"480 degC\"\nflow = \"if(eng{n}.n1 < 45, 4 kg/s, 0 kg/s)\"\navailable_when = \"{}\"\n", and_not(&format!("eng{n}.n3 > 30 && eng{n}.n1 < 45"), &hp_stuck)).unwrap();
        let cool = if fouled == "false" { "200 degC" } else { "260 degC" };
        writeln!(f, "[[pneumatic.valve]]\nid = \"prv{n}\"\nbetween = [\"bleed{n}\", \"{side}\"]\nopen_when = \"bleed{n}_pb.on && !fadec{n}.starter\"\nregulate = \"40 psi\"\ncool_to = \"{cool}\"\nstuck_when = \"{prv_stuck}\"\n").unwrap();
    }
    for (id, vol) in [("manifold_l", "0.3 m3"), ("manifold_r", "0.3 m3")] {
        writeln!(f, "[[pneumatic.duct]]\nid = \"{id}\"\nvolume = \"{vol}\"\n").unwrap();
    }
    switch(&mut f, "xbleed", false);
    f += "[[pneumatic.valve]]\nid = \"xbleed_valve\"\nbetween = [\"manifold_l\", \"manifold_r\"]\nopen_when = \"xbleed.on || apu_bleed_valve.open\"\nflow = \"6 kg/s\"\n\n";
    switch(&mut f, "apu_bleed_pb", true);
    writeln!(f, "[[pneumatic.duct]]\nid = \"apu_duct\"\nleak_when = \"{}\"\n", fails(c, "36_pneu.apu_bleed_duct", &["leak", "rupture"])).unwrap();
    f += "[[pneumatic.source]]\nid = \"apu_load_compressor\"\nduct = \"apu_duct\"\npressure = \"55 psi\"\ntemperature = \"220 degC\"\nflow = \"2.4 kg/s\"\navailable_when = \"apu.available\"\n\n";
    f += "[[pneumatic.valve]]\nid = \"apu_bleed_valve\"\nbetween = [\"apu_duct\", \"manifold_l\"]\nopen_when = \"apu_bleed_pb.on && apu.available\"\n\n";
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
    f += "# Brakes, tyres and steering from the plugin's landing gear (32_gear): 16 carbon brakes, the tyres it monitors, nose and body steering.\n\n";
    f += "[[input.number]]\nid = \"brake_pedals\"\n\n[[input.number]]\nid = \"tiller\"\n\n[[input.number]]\nid = \"rudder_pedals\"\n\n";
    for x in c.components.iter().filter(|x| x.id.starts_with("32_gear.wheel_") && x.id.ends_with("_brake")) {
        let id = &x.id["32_gear.".len()..];
        let circuit = if x.name.contains("wing") { "green" } else { "yellow" };
        writeln!(f, "# {}\n[[gear.brake]]\nid = \"{id}\"\ncircuit = \"{circuit}\"\npedal = \"brake_pedals.value\"\nmax_pressure = \"3000 psi\"\nair = \"air\"\n", x.name).unwrap();
    }
    for (key, id, _) in wheels(c) {
        writeln!(f, "[[gear.tyre]]\nid = \"{id}\"\npressure = \"{} psi\"\nair = \"air\"\n", if key.starts_with("nose") { 180 } else { 218 }).unwrap();
    }
    let steer = fails(c, "32_gear.steer_ctl", &["channel 1"]);
    let steer2 = fails(c, "32_gear.steer_ctl", &["channel 2"]);
    let powered = if steer == "false" { "true".to_owned() } else { format!("!(({steer}) && ({steer2}))") };
    writeln!(f, "[[gear.steering]]\nid = \"nose_steering\"\ncircuit = \"yellow\"\npowered_when = \"{powered}\"\ntiller = \"tiller.value\"\npedals = \"rudder_pedals.value\"\nair = \"air\"\ntowing_when = \"{}\"\n", fails(c, "32_gear.nose_steering", &["disconnect"])).unwrap();
    if comp(c, "32_gear.l_body_steering").is_some() {
        writeln!(f, "[[gear.steering]]\nid = \"body_steering\"\ncircuit = \"green\"\npowered_when = \"{powered}\"\ntiller = \"tiller.value\"\nair = \"air\"\ntiller_angle = 15\n").unwrap();
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
