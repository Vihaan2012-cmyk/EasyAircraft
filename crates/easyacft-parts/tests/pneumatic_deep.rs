use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const PSI: f64 = 6894.757;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("pneumatic", src);
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

const BLEED: &str = r#"
[[cockpit.switch]]
id = "bleed_on"

[[cockpit.switch]]
id = "fan_bad"

[[cockpit.switch]]
id = "rupture"

[[input.number]]
id = "hp"
initial = 60

[[pneumatic.duct]]
id = "port"

[[pneumatic.duct]]
id = "manifold"
overheat_temperature = "530 K"
leak_when = "rupture.on"

[[pneumatic.source]]
id = "eng"
duct = "port"
pressure = "hp.value * 1 psi"
temperature = "700 K"
flow = "3 kg/s"

[[pneumatic.valve]]
id = "prv"
between = ["port", "manifold"]
open_when = "bleed_on.on"
area = 0.005
open_time = "2 s"
actuation_pressure = "8 psi"
regulate = "44 psi"
cool_to = "473 K"
fan_air_when = "!fan_bad.on"

[[pneumatic.consumer]]
id = "pack"
duct = "manifold"
flow = "0.6 kg/s"
regulated = true
minimum = "20 psi"
"#;

#[test]
fn a_bleed_valve_travels_regulates_and_springs_shut_without_pressure() {
    let mut a = aircraft(BLEED);
    a.set("bleed_on.on", 1.0);
    run(&mut a, 1.0);
    let half = get(&a, "prv.position");
    assert!((0.4..0.6).contains(&half), "half way after 1 of 2 s: {half:.2}");
    run(&mut a, 10.0);
    let p = get(&a, "manifold.pressure") / PSI;
    assert!((41.0..47.0).contains(&p), "regulated to {p:.1} psi");
    assert!((get(&a, "pack.flow") - 0.6).abs() < 0.02, "the flow control valve holds the pack flow");
    a.set("hp.value", 5.0);
    run(&mut a, 3.0);
    assert_eq!(a.get("prv.position"), Some(0.0), "too little pressure to hold it open");
}

#[test]
fn the_precooler_needs_its_fan_air() {
    let mut a = aircraft(BLEED);
    a.set("bleed_on.on", 1.0);
    run(&mut a, 30.0);
    let t = get(&a, "prv.outlet_temperature");
    assert!((t - 473.0).abs() < 1.0, "fan-air valve holds {t:.0} K");
    assert_eq!(a.get("manifold.overheat"), Some(0.0), "{:.0} K", get(&a, "manifold.temperature"));
    a.set("fan_bad.on", 1.0);
    run(&mut a, 30.0);
    assert!(get(&a, "prv.outlet_temperature") > 650.0);
    assert_eq!(a.get("manifold.overheat"), Some(1.0), "{:.0} K", get(&a, "manifold.temperature"));
}

#[test]
fn a_duct_leak_is_detected_after_its_loop_delay() {
    let mut a = aircraft(BLEED);
    a.set("bleed_on.on", 1.0);
    run(&mut a, 10.0);
    a.set("rupture.on", 1.0);
    run(&mut a, 1.0);
    assert!(get(&a, "manifold.leak_flow") > 0.05);
    assert_eq!(a.get("manifold.leak_detected"), Some(0.0));
    run(&mut a, 3.0);
    assert_eq!(a.get("manifold.leak_detected"), Some(1.0));
}

const SOURCES: &str = r#"
[[input.number]]
id = "hp"
initial = 120

[[pneumatic.duct]]
id = "x"

[[pneumatic.duct]]
id = "y"

[[pneumatic.source]]
id = "strong"
duct = "x"
pressure = "hp.value * 1 psi"
temperature = "450 K"
flow = "2 kg/s"

[[pneumatic.source]]
id = "weak"
duct = "x"
pressure = "30 psi"
temperature = "450 K"
flow = "2 kg/s"

[[pneumatic.valve]]
id = "sov"
between = ["x", "y"]
overpressure = "85 psi"

[[pneumatic.consumer]]
id = "user"
duct = "y"
flow = "0.5 kg/s"
"#;

#[test]
fn check_valves_and_overpressure_shutoff() {
    let mut a = aircraft(SOURCES);
    run(&mut a, 5.0);
    assert_eq!(a.get("sov.open"), Some(0.0), "shut on overpressure: {:.0} psi downstream", get(&a, "y.pressure") / PSI);
    assert!(get(&a, "weak.flow") < 1e-3, "the weaker source sits behind its check valve");
    assert!(get(&a, "x.pressure") > 100.0 * PSI);
    a.set("hp.value", 60.0);
    run(&mut a, 10.0);
    assert_eq!(a.get("sov.open"), Some(1.0), "reopens once downstream falls 10 % below");
}
