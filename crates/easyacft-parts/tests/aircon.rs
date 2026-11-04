use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const PSI: f64 = 6894.757;
const FT: f64 = 0.3048;

const ACS: &str = r#"
[[environment.air]]
id = "air"
standard = true
isa_deviation = "15 K"

[[cockpit.switch]]
id = "packs_on"

[[cockpit.switch]]
id = "hot_air_on"
initial = true

[[cockpit.switch]]
id = "pack1_fail"

[[input.number]]
id = "pax"

[[input.number]]
id = "breach"

[[pneumatic.duct]]
id = "bleed"
volume = "0.1 m3"

[[pneumatic.source]]
id = "eng"
duct = "bleed"
pressure = "60 psi"
temperature = "470 K"
flow = "5 kg/s"

[[pneumatic.consumer]]
id = "pack1_draw"
duct = "bleed"
flow = "pack1.bleed_demand"
regulated = true
minimum = "5 psi"

[[pneumatic.consumer]]
id = "pack2_draw"
duct = "bleed"
flow = "pack2.bleed_demand"
regulated = true
minimum = "5 psi"

[[aircon.pack]]
id = "pack1"
duct = "bleed"
air = "air"
flow = "1.2 kg/s"
on_when = "packs_on.on && !acsc.inhibit"
flow_demand = "acsc.flow_factor"
outlet_demand = "acsc.pack_demand"
fcv_failed_when = "pack1_fail.on"

[[aircon.pack]]
id = "pack2"
duct = "bleed"
air = "air"
flow = "1.2 kg/s"
on_when = "packs_on.on && !acsc.inhibit"
flow_demand = "acsc.flow_factor"
outlet_demand = "acsc.pack_demand"

[[aircon.recirc_fan]]
id = "recirc"
flow = "0.8 kg/s"

[[aircon.mixer]]
id = "mixer"
packs = ["pack1", "pack2"]
recirculation = ["recirc"]
fuselage = "fus"
air = "air"
design_flow = "3.2 kg/s"

[[aircon.hot_air]]
id = "hot"
duct = "bleed"
fuselage = "fus"
on_when = "hot_air_on.on && acsc.hot_air_command"

[[aircon.trim_valve]]
id = "trim_fwd"
hot_air = "hot"

[[aircon.trim_valve]]
id = "trim_aft"
hot_air = "hot"

[[aircon.zone]]
id = "fwd"
mixer = "mixer"
fuselage = "fus"
air = "air"
share = 0.5
volume = "110 m3"
trim_valve = "trim_fwd"
occupants = "pax.value / 2"
selected = "297 K"
grid_cell = "0.5 m"
length = "8 m"

[[aircon.zone]]
id = "aft"
mixer = "mixer"
fuselage = "fus"
air = "air"
share = 0.5
volume = "110 m3"
trim_valve = "trim_aft"
occupants = "pax.value / 2"
selected = "295 K"

[[aircon.controller]]
id = "acsc"
packs = ["pack1", "pack2"]
zones = ["fwd", "aft"]
hot_air = ["hot"]

[[aircon.avionics_ventilation]]
id = "avncs"
air = "air"
fuselage = "fus"

[[aircon.cargo]]
id = "aft_cargo"
fuselage = "fus"
air = "air"
hot_air = "hot"

[[pressurization.fuselage]]
id = "fus"
air = "air"
volume = "240 m3"
mixers = ["mixer"]
breach_area = "breach.value"

[[pressurization.outflow_valve]]
id = "ofv"
fuselage = "fus"
area = 0.06

[[pressurization.safety_valve]]
id = "sv"
fuselage = "fus"

[[pressurization.controller]]
id = "cpc"
fuselage = "fus"
air = "air"
valves = ["ofv"]
"#;

fn aircraft() -> Aircraft {
    let (m, mut d) = load_str("aircon", ACS);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s * 50.0).round() as usize {
        a.step(0.02);
    }
}

fn get(a: &Aircraft, k: &str) -> f64 {
    a.get(k).unwrap_or_else(|| panic!("no {k}"))
}

fn all_finite(a: &Aircraft) -> Result<(), String> {
    for name in a.checked.slots.index.keys() {
        let v = a.get(name).unwrap_or(0.0);
        if !v.is_finite() {
            return Err(format!("{name} = {v}"));
        }
    }
    Ok(())
}

fn ground(a: &mut Aircraft) {
    a.set("air.on_ground", 1.0);
    a.set("air.altitude", 0.0);
    a.set("pax.value", 120.0);
    a.set("packs_on.on", 1.0);
}

#[test]
fn packs_cool_a_hot_cabin_on_the_ground() {
    let mut a = aircraft();
    ground(&mut a);
    run(&mut a, 30.0);
    assert_eq!(a.get("pack1.operating"), Some(1.0), "fault {}", get(&a, "pack1.fault"));
    let speed = get(&a, "pack1.acm_speed");
    assert!((0.3..1.15).contains(&speed), "ACM at {speed:.2} of rated");
    assert!(get(&a, "pack1.outlet_temperature") < get(&a, "fwd.temperature"), "the pack delivers cold air");
    run(&mut a, 1500.0);
    let (fwd, aft) = (get(&a, "fwd.head_temperature"), get(&a, "aft.temperature"));
    assert!((fwd - 297.0).abs() < 1.5 && (aft - 295.0).abs() < 1.5, "seated head height {fwd:.1} K forward, {aft:.1} K aft");
    assert!((get(&a, "fwd.temperature") - 297.0).abs() < 3.0, "forward zone mean {:.1} K", get(&a, "fwd.temperature"));
    assert!(aft < fwd, "each zone holds its own selection through its trim valve");
    assert_eq!(a.get("pack1.compressor_overheat"), Some(0.0), "{:.0} K", get(&a, "pack1.compressor_outlet_temperature"));
    let strat = get(&a, "fwd.stratification");
    assert!(strat.abs() < 4.0, "head-to-ankle {strat:.2} K in the resolved zone");
    assert!(get(&a, "fwd.co2") < 1500.0 && get(&a, "fwd.co2") > 420.0, "{:.0} ppm", get(&a, "fwd.co2"));
    assert!(get(&a, "fus.differential").abs() < 0.3 * PSI, "outflow valve open on the ground");
    assert!(get(&a, "avncs.equipment_temperature") < 340.0);
    all_finite(&a).unwrap();
}

#[test]
fn the_cabin_climbs_on_schedule_to_cruise() {
    let mut a = aircraft();
    ground(&mut a);
    run(&mut a, 300.0);
    a.set("air.on_ground", 0.0);
    a.set("air.airspeed", 150.0);
    a.set("air.vertical_speed", 10.0);
    let mut h = 0.0;
    let mut worst_rate: f64 = 0.0;
    while h < 10_668.0 {
        h += 0.2;
        a.set("air.altitude", h);
        a.step(0.02);
        if h > 1000.0 {
            worst_rate = worst_rate.max(get(&a, "fus.vertical_speed"));
        }
    }
    a.set("air.vertical_speed", 0.0);
    a.set("air.airspeed", 230.0);
    run(&mut a, 600.0);
    let cabin = get(&a, "fus.altitude") / FT;
    let diff = get(&a, "fus.differential") / PSI;
    assert!(cabin < 8200.0 && cabin > 4000.0, "cabin {cabin:.0} ft at FL350");
    assert!((7.0..8.75).contains(&diff), "differential {diff:.2} psi");
    assert!(worst_rate < 600.0 * FT / 60.0, "cabin climb held near its schedule: worst {:.0} ft/min", worst_rate / FT * 60.0);
    assert_eq!(a.get("cpc.mode"), Some(3.0), "cruise");
    assert_eq!(a.get("fus.excess_altitude"), Some(0.0));
    let (ofv, p1) = (get(&a, "ofv.position"), get(&a, "pack1.flow"));
    assert!(ofv > 0.01 && ofv < 0.9 && p1 > 0.5, "outflow valve modulating at {ofv:.2}, pack flow {p1:.2} kg/s");
    assert!((get(&a, "fwd.temperature") - 297.0).abs() < 3.0, "{:.1} K", get(&a, "fwd.temperature"));
    all_finite(&a).unwrap();
}

#[test]
fn losing_a_pack_puts_the_other_on_high_flow() {
    let mut a = aircraft();
    ground(&mut a);
    run(&mut a, 120.0);
    let normal = get(&a, "pack2.flow");
    a.set("pack1_fail.on", 1.0);
    run(&mut a, 30.0);
    assert_eq!(a.get("pack1.operating"), Some(0.0));
    assert_eq!(a.get("pack1.fault"), Some(6.0), "flow control valve fault");
    assert_eq!(a.get("acsc.mode"), Some(1.0), "single pack");
    assert!(get(&a, "pack2.flow") > 1.1 * normal, "{normal:.2} -> {:.2} kg/s", get(&a, "pack2.flow"));
}

#[test]
fn a_fuselage_breach_decompresses_without_blowing_up() {
    let mut a = aircraft();
    ground(&mut a);
    run(&mut a, 60.0);
    a.set("air.on_ground", 0.0);
    a.set("air.airspeed", 230.0);
    a.set("air.altitude", 10_668.0);
    run(&mut a, 900.0);
    a.set("breach.value", 0.5);
    run(&mut a, 2.0);
    assert_eq!(a.get("fus.rapid_decompression"), Some(1.0));
    run(&mut a, 60.0);
    assert_eq!(a.get("fus.excess_altitude"), Some(1.0));
    assert!(get(&a, "fus.differential").abs() < 0.5 * PSI, "the cabin has equalised");
    all_finite(&a).unwrap();
}

#[test]
fn nothing_goes_non_finite_under_abuse() {
    let mut a = aircraft();
    ground(&mut a);
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for k in 0..600 {
        if k % 20 == 0 {
            a.set("air.altitude", -500.0 + 16_000.0 * rnd());
            a.set("air.airspeed", 320.0 * rnd());
            a.set("air.on_ground", (rnd() > 0.5) as u8 as f64);
            a.set("packs_on.on", (rnd() > 0.2) as u8 as f64);
            a.set("pack1_fail.on", (rnd() > 0.8) as u8 as f64);
            a.set("pax.value", 400.0 * rnd());
            a.set("breach.value", if rnd() > 0.9 { 2.0 * rnd() } else { 0.0 });
            a.set("air.vertical_speed", 60.0 * (rnd() - 0.5));
        }
        let dt = if k % 7 == 0 { 2.0 } else { 0.02 + 0.3 * rnd() };
        a.step(dt);
        if let Err(e) = all_finite(&a) {
            panic!("step {k} (dt {dt:.2}): {e}");
        }
    }
    let t = get(&a, "fwd.temperature");
    assert!((180.0..420.0).contains(&t), "{t}");
}



#[test]
fn without_ozone_converters_a_polar_cruise_breaks_the_ozone_limit() {
    let src = ACS.replace("fcv_failed_when = \"pack1_fail.on\"", "fcv_failed_when = \"pack1_fail.on\"\nozone_converter = false").replace("outlet_demand = \"acsc.pack_demand\"\n\n[[aircon.recirc_fan]]", "outlet_demand = \"acsc.pack_demand\"\nozone_converter = false\n\n[[aircon.recirc_fan]]");
    let (m, mut d) = load_str("aircon", &src);
    let (mut bad, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
    let mut good = aircraft();
    for a in [&mut bad, &mut good] {
        ground(a);
        run(a, 60.0);
        a.set("air.on_ground", 0.0);
        a.set("air.airspeed", 230.0);
        a.set("air.latitude", 70.0);
        a.set("air.altitude", 10_668.0);
        run(a, 1800.0);
    }
    assert!(get(&good, "pack1.ozone") < 60.0, "converter: {:.0} ppb", get(&good, "pack1.ozone"));
    assert_eq!(good.get("fwd.ozone_high"), Some(0.0));
    assert!(get(&bad, "fwd.ozone") > 250.0, "{:.0} ppb sea-level equivalent", get(&bad, "fwd.ozone"));
    assert_eq!(bad.get("fwd.ozone_high"), Some(1.0));
}

#[test]
fn a_settled_cabin_is_comfortable_and_a_door_warns_while_it_holds_pressure() {
    let mut a = aircraft();
    ground(&mut a);
    run(&mut a, 1500.0);
    let vote = get(&a, "fwd.pmv");
    assert!(vote.abs() < 1.0 && get(&a, "fwd.ppd") < 30.0, "PMV {vote:.2}, {:.0} % dissatisfied", get(&a, "fwd.ppd"));
    let diff = get(&a, "fus.differential");
    assert_eq!(a.get("fus.residual_pressure"), Some((diff > 250.0) as u8 as f64), "{diff:.0} Pa");
}

#[test]
fn smoke_in_a_resolved_zone_reaches_its_detector() {
    let src = ACS.replace("grid_cell = \"0.5 m\"", "grid_cell = \"0.5 m\"\nsmoke_when = \"pax.value > 200\"\nsmoke_rate = \"0.2 g/s\"\nsmoke_at = 0.3");
    let (m, mut d) = load_str("aircon", &src);
    let (mut a, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
    ground(&mut a);
    run(&mut a, 120.0);
    assert_eq!(a.get("fwd.smoke_detected"), Some(0.0));
    a.set("pax.value", 250.0);
    let mut when = None;
    for s in 0..300 {
        run(&mut a, 1.0);
        if a.get("fwd.smoke_detected") == Some(1.0) {
            when = Some(s);
            break;
        }
    }
    let s = when.expect("detected");
    assert!(s >= 2, "smoke takes time to drift to the ceiling: {s} s");
    assert_eq!(a.get("aft.smoke_detected"), Some(0.0), "HEPA filters stop it reaching the other zone");
}

#[test]
fn moisture_freezes_on_the_skin_in_cruise_and_drips_after_landing_unless_it_is_dried() {
    let wet = ACS.replace("grid_cell = \"0.5 m\"", "").replace("selected = \"295 K\"", "selected = \"295 K\"\ninsulation_capacity = 0.0004");
    let dry = wet.replace("insulation_capacity = 0.0004", "insulation_capacity = 0.0004\ndrying_when = \"true\"");
    let mut runs = Vec::new();
    for src in [wet, dry] {
        let (m, mut d) = load_str("aircon", &src);
        let (mut a, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
        ground(&mut a);
        a.set("pax.value", 300.0);
        run(&mut a, 60.0);
        a.set("air.on_ground", 0.0);
        a.set("air.airspeed", 230.0);
        a.set("air.altitude", 10_668.0);
        for _ in 0..(2 * 3600 * 5) {
            a.step(0.2);
        }
        let ice = get(&a, "aft.insulation_ice");
        a.set("air.altitude", 0.0);
        a.set("air.airspeed", 0.0);
        a.set("air.on_ground", 1.0);
        let mut dripped = 0.0;
        for _ in 0..1800 {
            for _ in 0..5 {
                a.step(0.2);
                dripped += get(&a, "aft.drip") * 0.2;
            }
        }
        runs.push((ice, dripped));
        all_finite(&a).unwrap();
    }
    let ((ice, drip), (dried_ice, _)) = (runs[0], runs[1]);
    assert!(ice > 0.1, "{ice:.2} kg of ice behind the lining after two hours");
    assert!(drip > 0.0, "it rains in the cabin after landing");
    assert!(dried_ice < 0.2 * ice, "a zonal dryer keeps it dry: {dried_ice:.2} kg vs {ice:.2} kg");
}

#[test]
fn air_drifts_aft_along_the_cabin_and_takes_smoke_with_it() {
    let src = ACS
        .replace("grid_cell = \"0.5 m\"\nlength = \"8 m\"", "transfer_to = \"aft\"\ntransfer = 0.3\nsmoke_when = \"pax.value > 200\"\nsmoke_rate = \"0.2 g/s\"")
        .replace("selected = \"295 K\"", "selected = \"295 K\"\ntransfer_to = \"fwd\"\ntransfer = 0.0");
    let (m, mut d) = load_str("aircon", &src);
    let (mut a, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
    ground(&mut a);
    run(&mut a, 60.0);
    a.set("pax.value", 250.0);
    run(&mut a, 300.0);
    let (fwd, aft) = (get(&a, "fwd.smoke"), get(&a, "aft.smoke"));
    assert!(fwd > 1.0 && aft > 0.05 * fwd && aft < fwd, "smoke {fwd:.1} mg/m³ forward, {aft:.2} aft");
}

fn cruise(a: &mut Aircraft, seconds: f64) {
    ground(a);
    run(a, 60.0);
    a.set("air.on_ground", 0.0);
    a.set("air.airspeed", 230.0);
    a.set("air.altitude", 10_668.0);
    for _ in 0..(seconds * 10.0) as usize {
        a.step(0.1);
    }
}

#[test]
fn electric_cabin_air_compressors_replace_bleed() {
    let src = ACS
        .replace("[[aircon.pack]]\nid = \"pack1\"\nduct = \"bleed\"", "[[aircon.cabin_air_compressor]]\nid = \"cac1\"\nair = \"air\"\ndemand = \"pack1.bleed_demand\"\n\n[[aircon.cabin_air_compressor]]\nid = \"cac2\"\nair = \"air\"\ndemand = \"pack2.bleed_demand\"\n\n[[aircon.pack]]\nid = \"pack1\"\nduct = \"cac1\"")
        .replace("[[aircon.pack]]\nid = \"pack2\"\nduct = \"bleed\"", "[[aircon.pack]]\nid = \"pack2\"\nduct = \"cac2\"");
    assert!(src.contains("duct = \"cac2\""));
    let (m, mut d) = load_str("aircon", &src);
    let (mut a, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
    cruise(&mut a, 1200.0);
    assert_eq!(a.get("pack1.operating"), Some(1.0), "fault {}", get(&a, "pack1.fault"));
    let kw = get(&a, "cac1.power") / 1000.0;
    assert!((40.0..300.0).contains(&kw), "{kw:.0} kW of motor load at FL350");
    let diff = get(&a, "fus.differential") / PSI;
    assert!((7.0..8.75).contains(&diff), "pressurised on electric air: {diff:.2} psi");
    assert!(get(&a, "cac1.speed") > 0.8, "thin air needs the compressor near full speed");
    all_finite(&a).unwrap();
}

#[test]
fn a_duct_humidifier_lifts_cruise_humidity_and_cools_its_duct() {
    let wet = ACS.replace("[[aircon.controller]]", "[[aircon.humidifier]]\nid = \"hum\"\nzone = \"aft\"\ntarget = 0.2\n\n[[aircon.controller]]");
    let mut runs = Vec::new();
    for src in [ACS.to_string(), wet] {
        let (m, mut d) = load_str("aircon", &src);
        let (mut a, _) = Aircraft::from_model(m, &mut d).unwrap_or_else(|r| panic!("{}", r.render()));
        cruise(&mut a, 1800.0);
        runs.push((get(&a, "aft.humidity"), a.get("hum.water_used").unwrap_or(0.0)));
    }
    let ((dry, _), (humid, used)) = (runs[0], runs[1]);
    assert!(dry < 0.12, "cruise cabin air is dry: {:.0} %", dry * 100.0);
    assert!(humid > dry + 0.04, "humidified to {:.0} % from {:.0} %", humid * 100.0, dry * 100.0);
    assert!(used > 0.1, "{used:.2} kg of water");
}
