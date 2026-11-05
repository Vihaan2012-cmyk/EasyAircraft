use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const PSI: f64 = 6894.757;
const LITRE: f64 = 1e-3;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("hydraulic", src);
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

fn circuit(extra: &str) -> String {
    format!(
        r#"
[[cockpit.switch]]
id = "pump_on"

[[cockpit.switch]]
id = "use"

[[cockpit.switch]]
id = "leak"

[[hydraulic.circuit]]
id = "green"
nominal = "3000 psi"
fluid = "20 L"
leak_when = "leak.on"
leak_rate = "10 L/min"
{extra}

[[hydraulic.pump]]
id = "edp"
circuit = "green"
pressure = "3000 psi"
flow = "100 L/min"
running_when = "pump_on.on"

[[hydraulic.accumulator]]
id = "acc"
circuit = "green"
volume = "4 L"
precharge = "1000 psi"

[[hydraulic.accumulator]]
id = "brake_acc"
circuit = "green"
volume = "2 L"
precharge = "1000 psi"
check_valve = true

[[hydraulic.consumer]]
id = "act"
circuit = "green"
flow = "60 L/min"
moving_when = "use.on"
"#
    )
}

#[test]
fn accumulators_fill_from_the_reservoir_and_the_brake_one_holds_behind_its_check_valve() {
    let mut a = aircraft(&circuit(""));
    a.set("pump_on.on", 1.0);
    run(&mut a, 10.0);
    let p = get(&a, "green.pressure");
    let isothermal = |v: f64| v * (1.0 - 1000.0 * PSI / p);
    let hot = get(&a, "acc.fluid");
    assert!(hot < isothermal(4.0 * LITRE) - 0.05 * LITRE && get(&a, "acc.gas_temperature") > 295.0, "a fast charge heats the gas, so it takes less fluid: {:.2} L", hot / LITRE);
    run(&mut a, 600.0);
    let p = get(&a, "green.pressure");
    let (acc, brake) = (get(&a, "acc.fluid"), get(&a, "brake_acc.fluid"));
    assert!((acc - isothermal(4.0 * LITRE)).abs() < 0.05 * LITRE && (brake - isothermal(2.0 * LITRE)).abs() < 0.05 * LITRE, "cooled, the gas law: {:.2} L, {:.2} L", acc / LITRE, brake / LITRE);
    let level = get(&a, "green.fluid");
    let expect = 20.0 * LITRE - acc - brake - 1e-10 * p;
    assert!((level - expect).abs() < 0.15 * LITRE, "reservoir {:.2} L, expected {:.2} L less thermal swell", level / LITRE, expect / LITRE);
    a.set("pump_on.on", 0.0);
    a.set("leak.on", 1.0);
    run(&mut a, 90.0);
    assert!(get(&a, "green.pressure") < 300.0 * PSI && get(&a, "acc.fluid") < 0.05 * LITRE, "the circuit and its accumulator drain");
    assert!((get(&a, "brake_acc.fluid") - brake).abs() < 0.01 * brake && get(&a, "brake_acc.pressure") > 0.9 * p, "the brake accumulator keeps its charge: {:.0} psi", get(&a, "brake_acc.pressure") / PSI);
}

#[test]
fn a_leak_empties_the_reservoir_and_the_pump_cavitates() {
    let mut a = aircraft(&circuit(""));
    a.set("pump_on.on", 1.0);
    run(&mut a, 5.0);
    a.set("leak.on", 1.0);
    a.set("use.on", 1.0);
    let mut warned = None;
    for k in 0..240 {
        run(&mut a, 1.0);
        if warned.is_none() && a.get("green.low_level") == Some(1.0) {
            warned = Some(k);
            assert_eq!(a.get("green.available"), Some(1.0), "the level warning comes before the pressure goes");
        }
    }
    assert!(warned.is_some());
    assert!(get(&a, "green.fluid") < 0.01 * LITRE);
    assert_eq!(a.get("green.low"), Some(1.0), "{:.0} psi", get(&a, "green.pressure") / PSI);
}

#[test]
fn an_unpressurised_reservoir_at_altitude_starves_the_pump() {
    let mut good = aircraft(&circuit("ambient = \"20 kPa\""));
    let mut bad = aircraft(&circuit("ambient = \"20 kPa\"\nreservoir_pressure = \"0 psi\""));
    for a in [&mut good, &mut bad] {
        a.set("pump_on.on", 1.0);
        a.set("use.on", 1.0);
        run(a, 10.0);
    }
    assert_eq!((good.get("edp.cavitating"), bad.get("edp.cavitating")), (Some(0.0), Some(1.0)));
    assert!(get(&bad, "edp.flow") < get(&good, "edp.flow") && get(&bad, "green.pressure") < get(&good, "green.pressure") - 50.0 * PSI);
}

#[test]
fn fluid_heats_under_load_and_overheats_with_a_small_heat_exchanger() {
    let mut a = aircraft(&circuit("heat_exchanger = 20"));
    a.set("pump_on.on", 1.0);
    a.set("use.on", 1.0);
    run(&mut a, 60.0);
    let warm = get(&a, "green.temperature");
    assert!(warm > 290.0, "{warm:.1} K");
    let power = get(&a, "edp.power");
    let hydraulic = get(&a, "edp.flow") * get(&a, "green.pressure");
    let eff = hydraulic / power;
    assert!((0.7..0.92).contains(&eff), "shaft power covers the hydraulic power and the losses: {eff:.2}");
    run(&mut a, 900.0);
    assert_eq!(a.get("green.overheat"), Some(1.0), "{:.0} K", get(&a, "green.temperature"));
    assert!(get(&a, "edp.case_drain_flow") > 3.0 * get(&a, "edp.flow") * 0.03, "hot thin fluid leaks past the pistons");
}

const PRIORITY: &str = r#"
[[hydraulic.circuit]]
id = "c"
nominal = "3000 psi"
fluid = "20 L"

[[hydraulic.pump]]
id = "weak"
circuit = "c"
pressure = "3000 psi"
flow = "20 L/min"

[[hydraulic.consumer]]
id = "cargo_door"
circuit = "c"
flow = "40 L/min"
moving_when = "true"
priority_below = "2000 psi"

[[hydraulic.consumer]]
id = "flight_controls"
circuit = "c"
flow = "15 L/min"
moving_when = "true"
"#;

#[test]
fn a_priority_valve_keeps_pressure_for_the_flight_controls() {
    let mut a = aircraft(PRIORITY);
    run(&mut a, 10.0);
    let (door, fc) = (get(&a, "cargo_door.flow"), get(&a, "flight_controls.flow"));
    assert!(door < 0.7 * fc && a.get("flight_controls.powered") == Some(1.0), "door throttled to {:.1} of {:.1} L/min", door * 6e4, fc * 6e4);
    assert!(get(&a, "c.pressure") > 1880.0 * PSI && get(&a, "c.pressure") < 2100.0 * PSI, "held near the priority setting: {:.0} psi", get(&a, "c.pressure") / PSI);
    let p = get(&a, "c.pressure");
    run(&mut a, 1.0);
    assert!((get(&a, "c.pressure") - p).abs() < 5.0 * PSI, "steady, not chattering");
}

const PTU: &str = r#"
[[hydraulic.circuit]]
id = "green"
nominal = "3000 psi"
fluid = "20 L"
heat_exchanger = 1e6

[[hydraulic.circuit]]
id = "yellow"
nominal = "3000 psi"
fluid = "15 L"
heat_exchanger = 1e6

[[hydraulic.pump]]
id = "edp"
circuit = "green"
pressure = "3000 psi"
flow = "100 L/min"

[[hydraulic.consumer]]
id = "yellow_user"
circuit = "yellow"
flow = "20 L/min"
moving_when = "true"

[[hydraulic.ptu]]
id = "ptu"
between = ["green", "yellow"]
flow = "40 L/min"
"#;

#[test]
fn a_ptu_moves_power_not_fluid() {
    let mut a = aircraft(PTU);
    run(&mut a, 10.0);
    assert_eq!(a.get("ptu.running"), Some(1.0));
    let y = get(&a, "yellow.pressure") / PSI;
    assert!(y > 2000.0, "yellow driven to {y:.0} psi");
    let (gf, yf) = (get(&a, "green.fluid"), get(&a, "yellow.fluid"));
    run(&mut a, 30.0);
    assert!((get(&a, "green.fluid") - gf).abs() < 0.01 * LITRE && (get(&a, "yellow.fluid") - yf).abs() < 0.01 * LITRE, "no fluid crosses");
    let (out, pump) = (get(&a, "ptu.power"), get(&a, "edp.power"));
    assert!(out > 0.0 && pump * 0.85 > out / 0.8, "green pays for yellow's power and the losses: {pump:.0} W for {out:.0} W");
    assert!(get(&a, "ptu.flow") <= 40.0 * LITRE / 60.0 + 1e-6);
}
