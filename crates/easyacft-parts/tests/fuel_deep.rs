use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("fuel", src);
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

fn fuel(wing: &str, cooling: &str) -> Aircraft {
    aircraft(&format!(
        r#"
[[cockpit.switch]]
id = "pump_on"
initial = true

[[cockpit.switch]]
id = "xfr"

[[cockpit.switch]]
id = "leak"

[[cockpit.switch]]
id = "suck"

[[cockpit.switch]]
id = "burning"
initial = true

[[input.number]]
id = "skin"
initial = 288.15

[[fuel.tank]]
id = "wing"
capacity = "5000 kg"
quantity = "{wing}"
unusable = "50 kg"
skin_temperature = "skin.value * 1 K"
cooling = {cooling}

[[fuel.tank]]
id = "trim"
capacity = "1000 kg"
quantity = "990 kg"
overflow_to = "surge"

[[fuel.tank]]
id = "surge"
capacity = "200 kg"

[[fuel.line]]
id = "feed_line"
leak_when = "leak.on"

[[fuel.pump]]
id = "boost"
tank = "wing"
outlet = "feed_line"
flow = "2 kg/s"
running_when = "pump_on.on"

[[fuel.pump]]
id = "xfr_pump"
tank = "wing"
outlet = "trim"
flow = "1 kg/s"
running_when = "xfr.on"

[[fuel.valve]]
id = "gravity"
between = ["wing", "feed_line"]
check = true

[[fuel.feed]]
id = "eng"
line = "feed_line"
demand = "if(burning.on, 0.5 kg/s, 0 kg/s)"
suction_when = "suck.on"
"#
    ))
}

#[test]
fn a_cold_soaked_tank_waxes_and_starves_its_pump() {
    let mut a = fuel("4000 kg", "20000");
    a.set("skin.value", 220.0);
    run(&mut a, 120.0);
    assert_eq!(a.get("eng.starved"), Some(0.0));
    let cooling = get(&a, "wing.temperature");
    assert!(cooling < 288.0 && cooling > 233.0, "{cooling:.1} K");
    run(&mut a, 900.0);
    assert_eq!(a.get("wing.frozen"), Some(1.0), "{:.1} K", get(&a, "wing.temperature"));
    assert!(get(&a, "boost.flow") < 0.1, "waxed fuel will not pump");
    assert_eq!(a.get("eng.starved"), Some(1.0));
    assert!(get(&a, "wing.volume") < get(&a, "wing.quantity") / 804.0, "cold fuel is denser");
}

#[test]
fn an_overfilled_tank_spills_into_its_surge_tank() {
    let mut a = fuel("4000 kg", "0");
    let total = |a: &Aircraft| get(a, "wing.quantity") + get(a, "trim.quantity") + get(a, "surge.quantity");
    let start = total(&a);
    a.set("burning.on", 0.0);
    a.set("xfr.on", 1.0);
    run(&mut a, 60.0);
    assert_eq!(get(&a, "trim.quantity"), 1000.0);
    let surge = get(&a, "surge.quantity");
    assert!(surge > 40.0, "{surge:.1} kg vented to the surge tank");
    assert!((total(&a) - start).abs() < 0.5, "no fuel is created or lost: {:.2} kg", total(&a) - start);
    assert_eq!(get(&a, "trim.spilled"), 0.0);
}

#[test]
fn unusable_fuel_stays_in_the_tank() {
    let mut a = fuel("80 kg", "0");
    run(&mut a, 120.0);
    let q = get(&a, "wing.quantity");
    assert!((q - 50.0).abs() < 1.0, "{q:.1} kg left");
    assert_eq!((a.get("wing.empty"), a.get("boost.running"), a.get("eng.starved")), (Some(1.0), Some(0.0), Some(1.0)));
}

#[test]
fn the_engine_pump_suction_feeds_through_the_gravity_valve() {
    let mut a = fuel("4000 kg", "0");
    a.set("pump_on.on", 0.0);
    run(&mut a, 2.0);
    assert_eq!(a.get("eng.starved"), Some(1.0), "no boost pressure");
    a.set("suck.on", 1.0);
    let before = get(&a, "wing.quantity");
    run(&mut a, 10.0);
    assert_eq!(a.get("eng.starved"), Some(0.0));
    let used = before - get(&a, "wing.quantity");
    assert!((used - 10.0 * get(&a, "eng.flow")).abs() < 0.5, "the wing tank pays for it: {used:.2} kg");
}

#[test]
fn a_check_valve_stops_reverse_flow_and_a_line_leak_loses_fuel() {
    let mut a = fuel("4000 kg", "0");
    run(&mut a, 2.0);
    assert_eq!(a.get("gravity.open"), Some(0.0), "boost pressure in the line holds the check valve shut");
    a.set("burning.on", 0.0);
    let before = get(&a, "wing.quantity");
    a.set("leak.on", 1.0);
    run(&mut a, 10.0);
    let leak = get(&a, "feed_line.leak_flow");
    assert!(leak > 0.05);
    assert!((before - get(&a, "wing.quantity") - 10.0 * leak).abs() < 0.5);
}
