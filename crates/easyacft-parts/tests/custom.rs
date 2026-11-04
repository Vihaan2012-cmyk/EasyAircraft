use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const SRC: &str = r#"
[kinds.heater_control]
help = "Holds a cabin zone's temperature with a heater"
settings.target = { type = "degC", default = "22 degC" }
settings.gain = "number"
settings.enabled = { type = "expr bool", default = "true" }
settings.bus = "part electrical.bus"
values.demand = "W"
values.zone_temp = "K"

[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "heat_sw"

[[electrical.bus]]
id = "ac"
nominal = "115 V"

[[electrical.ext_power]]
id = "gpu"
bus = "ac"
voltage = "115 V"
rating = "90 kVA"
connected_when = "true"

[[custom.heater_control]]
id = "zone1"
gain = 500
enabled = "heat_sw.on"
bus = "ac"

[[electrical.load]]
id = "heater"
bus = "ac"
power = "zone1.demand"
"#;

fn aircraft(src: &str) -> Result<Aircraft, String> {
    let (m, mut d) = load_str("custom", src);
    Aircraft::from_model(m, &mut d).map(|(a, _)| a).map_err(|r| r.render())
}

#[test]
fn a_custom_part_drives_a_load_through_its_values() {
    let mut a = aircraft(SRC).unwrap();
    let n = a.on_kind("heater_control", |p| {
        let on = p.get("enabled") != 0.0 && p.read(&format!("{}.powered", p.parts("bus")[0])) == Some(1.0);
        let t = if p.get("zone_temp") == 0.0 { 283.15 } else { p.get("zone_temp") };
        let demand = if on { (p.setting("gain") * (p.setting("target") - t)).clamp(0.0, 10_000.0) } else { 0.0 };
        p.set("demand", demand);
        p.set("zone_temp", t + demand * 1e-4 * p.dt);
    });
    assert_eq!(n, 1);
    for _ in 0..60 {
        a.step(1.0 / 60.0);
    }
    assert_eq!(a.get("heater.power"), Some(0.0), "switch off");
    a.set("heat_sw.on", 1.0);
    for _ in 0..60 {
        a.step(1.0 / 60.0);
    }
    let p = a.get("heater.power").unwrap();
    assert!((4_000.0..=10_500.0).contains(&p), "heater draws what zone1 asks: {p}");
    assert!(a.get("gpu.load").unwrap() > 3_000.0);
    a.set("zone1.failed", 1.0);
    let before = a.get("zone1.demand").unwrap();
    for _ in 0..30 {
        a.step(1.0 / 60.0);
    }
    assert_eq!(a.get("zone1.demand"), Some(before), "a failed part is not stepped");
}

#[test]
fn bad_kind_declarations_are_reported() {
    let bad = SRC.replace("values.demand = \"W\"", "values.demand = \"watts\"").replace("gain = 500\n", "");
    let e = aircraft(&bad).err().expect("errors");
    assert!(e.contains("E0013") && e.contains("`watts` is not a type"), "{e}");
    let missing = SRC.replace("gain = 500\n", "");
    let e = aircraft(&missing).err().expect("errors");
    assert!(e.contains("gain"), "{e}");
}

const NET: &str = r#"
[kinds.vfg]
help = "A variable-frequency generator with its own regulator"
settings.drive_rpm = "expr rpm"
settings.droop = "ohm"
values.frequency = "Hz"
ports.out = "electrical source"

[kinds.ssr]
help = "A solid-state contactor"
settings.closed = "expr bool"
links.poles = "electrical"

[kinds.ram_pump]
ports.out = "hydraulic source"
settings.on = "expr bool"

[[cockpit.switch]]
id = "tie_sw"

[[electrical.bus]]
id = "gen_bus"
nominal = "115 V"

[[electrical.bus]]
id = "far_bus"
nominal = "115 V"

[[custom.vfg]]
id = "g1"
drive_rpm = "12000 rpm"
droop = "0.05 ohm"
out = "gen_bus"

[[custom.ssr]]
id = "k1"
closed = "tie_sw.on"
poles = ["gen_bus", "far_bus"]

[[electrical.load]]
id = "galley"
bus = "far_bus"
power = "20 kW"

[[hydraulic.circuit]]
id = "blue"
nominal = "3000 psi"
fluid = "10 L"

[[custom.ram_pump]]
id = "rat"
out = "blue"
on = "true"
"#;

#[test]
fn custom_parts_join_the_networks_through_ports_and_links() {
    let mut a = aircraft(NET).unwrap();
    a.on_kind("vfg", |p| {
        p.set("frequency", p.get("drive_rpm") * 2.0);
        let v = p.level("out");
        let e = 115.0 + (115.0 - v) * 0.5;
        p.drive("out", e, 1.0 / p.setting("droop"));
    });
    a.on_kind("ssr", |p| {
        let g = if p.get("closed") != 0.0 { 1000.0 } else { 0.0 };
        p.conduct("poles", g);
    });
    a.on_kind("ram_pump", |p| {
        p.drive("out", 3000.0 * 6894.757, 30.0 / 60_000.0 / (300.0 * 6894.757));
    });
    for _ in 0..120 {
        a.step(1.0 / 60.0);
    }
    let gv = a.get("gen_bus.voltage").unwrap();
    assert!((gv - 115.0).abs() < 3.0, "the custom generator holds its bus: {gv}");
    assert_eq!(a.get("far_bus.powered"), Some(0.0), "open contactor");
    a.set("tie_sw.on", 1.0);
    for _ in 0..60 {
        a.step(1.0 / 60.0);
    }
    assert_eq!(a.get("far_bus.powered"), Some(1.0));
    let i = a.get("g1.out_q").unwrap();
    assert!((150.0..200.0).contains(&i), "20 kW at 115 V through the custom link: {i} A");
    assert!(a.get("k1.poles_q").unwrap() > 150.0);
    let p = a.get("blue.pressure").unwrap() / 6894.757;
    assert!(p > 2500.0, "the custom pump pressurises its circuit: {p} psi");
}
