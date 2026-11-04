use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const TWIN: &str = r#"
[aircraft]
name = "Test Twin"

[[cockpit.switch]]
id = "sw_bat"
initial = true

[[cockpit.switch]]
id = "sw_gen{n}"
count = 2
initial = true

[[cockpit.switch]]
id = "eng{n}_running"
count = 2

[[electrical.bus]]
id = "ac{n}"
count = 2
nominal = "115 V"

[[electrical.bus]]
id = "dc{n}"
count = 2
nominal = "28 V"

[[electrical.bus]]
id = "hot_bat"
nominal = "28 V"

[[electrical.battery]]
id = "bat"
bus = "hot_bat"
voltage = "24 V"
capacity = "44 Ah"

[[electrical.contactor]]
id = "blc"
between = ["hot_bat", "dc1"]
close_when = "sw_bat.on"

[[electrical.generator]]
id = "gen{n}"
count = 2
bus = "ac{n}"
voltage = "115 V"
frequency = "400 Hz"
rating = "60 kVA"
driven_when = "eng{n}_running.on"

[[electrical.bus]]
id = "gen{n}_out"
count = 2
nominal = "115 V"

[[electrical.contactor]]
id = "btc"
between = ["ac1", "ac2"]
close_when = "!(gen1.ready && gen2.ready)"

[[electrical.tru]]
id = "tr{n}"
count = 2
input = "ac{n}"
output = "dc{n}"
voltage = "28 V"
rating = "6 kW"

[[electrical.contactor]]
id = "galley_feed"
between = ["ac2", "galley_bus"]

[[electrical.bus]]
id = "galley_bus"
nominal = "115 V"

[[electrical.load]]
id = "galley"
bus = "galley_bus"
power = "48 kW"
shed_priority = 5

[[electrical.load]]
id = "cabin"
bus = "ac1"
power = "15 kW"

[[electrical.bus]]
id = "oven_bus"
nominal = "115 V"

[[electrical.breaker]]
id = "cb_oven"
between = ["ac1", "oven_bus"]
rating = "100 A"

[[electrical.load]]
id = "avionics"
bus = "dc1"
power = "1500 W"

[[cockpit.switch]]
id = "sw_oven"

[[electrical.load]]
id = "oven"
bus = "oven_bus"
power = "30 kW"
on_when = "sw_oven.on"

[[logic.flag]]
id = "dc1_low"
when = "delay(dc1.voltage < 25 V, 2 s)"
"#;

fn twin() -> Aircraft {
    let (model, mut d) = load_str("twin", TWIN);
    match Aircraft::from_model(model, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s * 60.0) as usize {
        a.step(1.0 / 60.0);
    }
}

#[test]
fn a_cold_aircraft_runs_on_its_battery_then_the_generators_take_over() {
    let mut a = twin();
    run(&mut a, 1.0);
    let dc1 = a.get("dc1.voltage").unwrap();
    assert!((20.0..27.0).contains(&dc1), "dc1 on battery {dc1}");
    assert!(a.get("bat.current").unwrap() > 20.0, "battery supplies the avionics");
    assert_eq!(a.get("ac1.powered"), Some(0.0));
    a.set("eng1_running.on", 1.0);
    a.set("eng2_running.on", 1.0);
    run(&mut a, 1.0);
    assert!((a.get("ac1.voltage").unwrap() - 115.0).abs() < 3.0);
    assert!((a.get("dc1.voltage").unwrap() - 28.0).abs() < 1.5, "TR feeds dc1: {}", a.get("dc1.voltage").unwrap());
    assert!(a.get("bat.current").unwrap() < 0.0, "the battery charges: {}", a.get("bat.current").unwrap());
    assert_eq!(a.get("btc.closed"), Some(0.0));
}

#[test]
fn losing_a_generator_closes_the_tie_and_sheds_the_galley() {
    let mut a = twin();
    a.set("eng1_running.on", 1.0);
    a.set("eng2_running.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("galley.powered"), Some(1.0));
    a.set("gen2.failed", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("btc.closed"), Some(1.0));
    assert!(a.get("ac2.powered").unwrap() != 0.0, "ac2 fed through the tie");
    assert_eq!(a.get("galley.shed"), Some(1.0), "one 60 kVA generator cannot carry the galley as well");
    a.set("gen2.failed", 0.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("galley.shed"), Some(0.0));
}

#[test]
fn an_overloaded_breaker_trips_and_resets_when_pulled() {
    let mut a = twin();
    a.set("eng1_running.on", 1.0);
    a.set("eng2_running.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("cb_oven.tripped"), Some(0.0));
    a.set("sw_oven.on", 1.0);
    run(&mut a, 0.5);
    assert!(a.get("cb_oven.current").unwrap() > 200.0, "30 kW at 115 V is over the 100 A rating");
    run(&mut a, 3.0);
    assert_eq!(a.get("cb_oven.tripped"), Some(1.0));
    assert_eq!(a.get("oven.powered"), Some(0.0));
    a.set("sw_oven.on", 0.0);
    a.set("cb_oven.pulled", 1.0);
    run(&mut a, 0.1);
    assert_eq!(a.get("cb_oven.tripped"), Some(0.0));
}

#[test]
fn timers_in_files_and_snapshots_work() {
    let mut a = twin();
    a.set("sw_bat.on", 0.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("dc1_low.on"), Some(0.0));
    run(&mut a, 1.5);
    assert_eq!(a.get("dc1_low.on"), Some(1.0));
    let snap = a.snapshot();
    let bytes = snap.to_bytes();
    a.set("sw_bat.on", 1.0);
    run(&mut a, 1.0);
    let back = easyacft_core::Snapshot::from_bytes(&bytes).unwrap();
    assert!(a.restore(&back));
    assert_eq!(a.get("sw_bat.on"), Some(0.0));
    assert_eq!(a.get("dc1_low.on"), Some(1.0));
}

#[test]
fn wrong_units_and_names_are_caught_before_running() {
    let bad = TWIN.replace("rating = \"6 kW\"", "rating = \"6 psi\"").replace("close_when = \"sw_bat.on\"", "close_when = \"sw_batt.on\"");
    let (model, mut d) = load_str("bad", &bad);
    let r = Aircraft::from_model(model, &mut d).err().expect("errors");
    let text = r.render();
    assert!(text.contains("error[E0103]: `rating` must be power"), "{text}");
    assert!(text.contains("this is pressure"), "{text}");
    assert!(text.contains("error[E0201]: nothing called `sw_batt.on`"), "{text}");
    assert!(text.contains("did you mean `sw_bat.on`?"), "{text}");
}

/// The share of charge below which a battery's voltage collapses (electrical.rs KNEE).
const KNEE_CHARGE: f64 = 0.1;

const FLAT: &str = r#"
[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.battery]]
id = "bat"
bus = "dc"
voltage = "28 V"
capacity = "0.05 Ah"
charge = 0.3

[[electrical.load]]
id = "radio"
bus = "dc"
power = "200 W"

[[electrical.load]]
id = "light"
bus = "dc"
power = "100 W"
min_voltage = "26 V"
"#;

fn flat() -> Aircraft {
    let (model, mut d) = load_str("flat", FLAT);
    match Aircraft::from_model(model, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

#[test]
fn a_flat_battery_dies_and_its_bus_goes_dead() {
    let mut a = flat();
    run(&mut a, 1.0);
    assert_eq!(a.get("dc.powered"), Some(1.0));
    run(&mut a, 120.0);
    assert!(a.get("bat.charge").unwrap() < KNEE_CHARGE, "charge {}", a.get("bat.charge").unwrap());
    assert!(a.get("dc.voltage").unwrap() < 0.8 * 28.0, "past the knee its voltage falls away: {}", a.get("dc.voltage").unwrap());
    assert_eq!(a.get("dc.powered"), Some(0.0), "and the bus goes dead, with nothing left drawing on it");
    assert_eq!(a.get("radio.powered"), Some(0.0));
}

#[test]
fn loads_drop_out_below_their_minimum_voltage_and_draw_what_the_bus_gives_them() {
    let mut a = flat();
    run(&mut a, 0.5);
    let v = a.get("dc.voltage").unwrap();
    assert!(v < 26.0 && v > 14.0, "the 30 % battery sags the bus: {v:.1} V");
    assert_eq!(a.get("light.powered"), Some(0.0), "below 26 V the light switches itself off");
    assert_eq!(a.get("light.power"), Some(0.0));
    let p = a.get("radio.power").unwrap();
    assert!((p - 200.0).abs() < 20.0, "constant power holds near nominal draw: {p:.0} W");
}
