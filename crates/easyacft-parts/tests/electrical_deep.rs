use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("electrical", src);
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

const GEN: &str = r#"
[[cockpit.switch]]
id = "drive"

[[cockpit.switch]]
id = "short"

[[cockpit.switch]]
id = "cooler_bad"

[[input.number]]
id = "spd"
initial = 1

[[input.number]]
id = "demand"

[[electrical.bus]]
id = "ac"
nominal = "115 V"
short_when = "short.on"

[[electrical.generator]]
id = "gen"
bus = "ac"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
driven_when = "drive.on"
speed = "spd.value"
variable_frequency = true
min_speed = 0.6
max_speed = 1.2
overload_time = "10 s"
oil_cooled_when = "!cooler_bad.on"

[[electrical.load]]
id = "load"
bus = "ac"
power = "demand.value * 1 kW"
"#;

fn cycle(a: &mut Aircraft) {
    a.set("drive.on", 0.0);
    run(a, 0.1);
    a.set("drive.on", 1.0);
    run(a, 0.5);
}

#[test]
fn a_generator_excites_follows_its_drive_and_regulates_out_droop() {
    let mut a = aircraft(GEN);
    a.set("drive.on", 1.0);
    run(&mut a, 0.1);
    assert_eq!(a.get("gen.ready"), Some(0.0), "the field is still building");
    run(&mut a, 0.3);
    assert_eq!(a.get("gen.ready"), Some(1.0));
    a.set("spd.value", 0.9);
    run(&mut a, 0.1);
    assert!((get(&a, "ac.frequency") - 360.0).abs() < 1e-6, "variable frequency follows speed");
    a.set("spd.value", 0.5);
    run(&mut a, 0.1);
    assert_eq!((a.get("gen.ready"), a.get("ac.frequency")), (Some(0.0), Some(0.0)), "under speed, off line");
    a.set("spd.value", 1.0);
    a.set("demand.value", 80.0);
    run(&mut a, 0.5);
    let sag = get(&a, "ac.voltage");
    run(&mut a, 4.0);
    let held = get(&a, "ac.voltage");
    assert!(sag < 114.0 && (held - 115.0).abs() < 0.3, "droop {sag:.1} V trimmed back to {held:.1} V");
}

#[test]
fn its_control_unit_trips_on_overspeed_overload_and_a_short() {
    let mut a = aircraft(GEN);
    a.set("drive.on", 1.0);
    run(&mut a, 0.5);
    a.set("spd.value", 1.4);
    run(&mut a, 0.5);
    assert_eq!(a.get("gen.tripped"), Some(0.0), "not for half a second");
    run(&mut a, 1.0);
    assert_eq!((a.get("gen.tripped"), a.get("gen.fault"), a.get("gen.ready")), (Some(1.0), Some(3.0), Some(0.0)), "overfrequency");
    a.set("spd.value", 1.0);
    cycle(&mut a);
    assert_eq!((a.get("gen.tripped"), a.get("gen.ready")), (Some(0.0), Some(1.0)), "switching it off and on clears the trip");
    a.set("demand.value", 180.0);
    run(&mut a, 3.0);
    assert_eq!(a.get("gen.tripped"), Some(0.0));
    run(&mut a, 3.0);
    assert_eq!(a.get("gen.fault"), Some(5.0), "200 % trips well inside the 150 % time");
    let mut b = aircraft(GEN);
    b.set("drive.on", 1.0);
    run(&mut b, 0.5);
    b.set("short.on", 1.0);
    run(&mut b, 1.0);
    assert_eq!(b.get("gen.fault"), Some(6.0), "fault current trips it inside a second");
    assert_eq!(b.get("ac.powered"), Some(0.0));
}

#[test]
fn drive_oil_overheats_without_its_cooler() {
    let mut a = aircraft(GEN);
    a.set("drive.on", 1.0);
    a.set("demand.value", 70.0);
    run(&mut a, 600.0);
    let cool = get(&a, "gen.oil_temperature");
    assert!((370.0..400.0).contains(&cool) && a.get("gen.oil_overheat") == Some(0.0), "oil {cool:.0} K at 78 %");
    a.set("cooler_bad.on", 1.0);
    run(&mut a, 600.0);
    assert_eq!(a.get("gen.oil_overheat"), Some(1.0), "oil {:.0} K", get(&a, "gen.oil_temperature"));
}

fn battery(ambient: &str, extra: &str) -> Aircraft {
    aircraft(&format!(
        r#"
[[input.number]]
id = "demand"

[[electrical.bus]]
id = "dc"
nominal = "24 V"

[[electrical.battery]]
id = "bat"
bus = "dc"
voltage = "24 V"
capacity = "40 Ah"
ambient = "{ambient}"
{extra}

[[electrical.load]]
id = "load"
bus = "dc"
power = "demand.value * 1 W"
type = "resistive"
min_voltage = "1 V"
"#
    ))
}

#[test]
fn a_cold_battery_sags_and_holds_less() {
    let mut warm = battery("288.15 K", "");
    let mut cold = battery("253.15 K", "");
    for a in [&mut warm, &mut cold] {
        a.set("demand.value", 2000.0);
        run(a, 60.0);
    }
    let (vw, vc) = (get(&warm, "bat.voltage"), get(&cold, "bat.voltage"));
    assert!(vc < vw - 0.3, "cold {vc:.2} V vs warm {vw:.2} V");
    let (rw, rc) = (get(&warm, "bat.remaining"), get(&cold, "bat.remaining"));
    assert!(rc < 0.8 * rw, "cold holds {:.1} Ah vs {:.1} Ah", rc / 3600.0, rw / 3600.0);
    assert!(get(&warm, "bat.temperature") > 288.15, "I²R warms it");
}

#[test]
fn heavy_current_costs_more_charge_than_it_delivers() {
    let mut hi = battery("288.15 K", "");
    let mut lo = battery("288.15 K", "");
    hi.set("demand.value", 4000.0);
    lo.set("demand.value", 400.0);
    let (mut ah_hi, mut ah_lo) = (0.0, 0.0);
    for _ in 0..3000 {
        hi.step(0.02);
        ah_hi += get(&hi, "bat.current") * 0.02;
    }
    for _ in 0..30000 {
        lo.step(0.02);
        ah_lo += get(&lo, "bat.current") * 0.02;
    }
    let per_hi = (1.0 - get(&hi, "bat.charge")) / ah_hi;
    let per_lo = (1.0 - get(&lo, "bat.charge")) / ah_lo;
    assert!(per_hi > per_lo * 1.03, "Peukert: {per_hi:.3e} vs {per_lo:.3e} charge per coulomb");
}

#[test]
fn an_unlimited_charge_in_a_hot_bay_runs_away() {
    let mut a = battery(
        "340 K",
        "charge = 0.2\ncharge_limit = \"2000 A\"\ncooling = 1\n\n[[electrical.ext_power]]\nid = \"gpu\"\nbus = \"dc\"\nvoltage = \"29 V\"\nrating = \"40 kW\"\nconnected_when = \"true\"",
    );
    run(&mut a, 30.0);
    assert!(get(&a, "bat.current") < -100.0, "charging hard: {:.0} A", get(&a, "bat.current"));
    let mut overheat_at = None;
    for k in 0..4000 {
        run(&mut a, 1.0);
        if overheat_at.is_none() && a.get("bat.overheat") == Some(1.0) {
            overheat_at = Some(k);
        }
        if a.get("bat.current") == Some(0.0) {
            break;
        }
    }
    assert!(overheat_at.is_some(), "overheat warns first");
    assert_eq!(a.get("bat.runaway"), Some(1.0));
    assert_eq!(a.get("bat.current"), Some(0.0), "vented open circuit at {:.0} K", get(&a, "bat.temperature"));
}

const SWITCHING: &str = r#"
[[cockpit.switch]]
id = "main_on"

[[cockpit.switch]]
id = "aux_on"

[[cockpit.switch]]
id = "heater_on"

[[cockpit.switch]]
id = "new_fuse"

[[cockpit.switch]]
id = "pump_on"

[[electrical.bus]]
id = "main"
nominal = "28 V"

[[electrical.bus]]
id = "ess"
nominal = "28 V"

[[electrical.bus]]
id = "heater_bus"
nominal = "28 V"

[[electrical.ext_power]]
id = "main_src"
bus = "main"
voltage = "28 V"
rating = "10 kW"
connected_when = "main_on.on"

[[electrical.ext_power]]
id = "aux_src"
bus = "ess"
voltage = "28.5 V"
rating = "10 kW"
connected_when = "aux_on.on"

[[electrical.diode]]
id = "d"
between = ["main", "ess"]

[[electrical.breaker]]
id = "fuse"
between = ["main", "heater_bus"]
rating = "10 A"
fuse = true
replaced_when = "new_fuse.on"

[[electrical.load]]
id = "heater"
bus = "heater_bus"
power = "1 kW"
on_when = "heater_on.on"
type = "resistive"

[[electrical.load]]
id = "pump"
bus = "ess"
power = "500 W"
on_when = "pump_on.on"
type = "motor"
inrush = 6
start_time = "1 s"
"#;

#[test]
fn a_diode_feeds_one_way_only() {
    let mut a = aircraft(SWITCHING);
    a.set("main_on.on", 1.0);
    run(&mut a, 0.2);
    assert_eq!((a.get("ess.powered"), a.get("d.conducting")), (Some(1.0), Some(1.0)));
    a.set("main_on.on", 0.0);
    a.set("aux_on.on", 1.0);
    run(&mut a, 0.2);
    assert_eq!(a.get("d.conducting"), Some(0.0));
    assert_eq!(a.get("main.powered"), Some(0.0), "no back-feed");
    assert!(get(&a, "d.current").abs() < 1.0);
}

#[test]
fn a_blown_fuse_stays_blown_until_replaced() {
    let mut a = aircraft(SWITCHING);
    a.set("main_on.on", 1.0);
    a.set("heater_on.on", 1.0);
    run(&mut a, 5.0);
    assert_eq!(a.get("fuse.tripped"), Some(1.0), "36 A through a 10 A fuse");
    a.set("heater_on.on", 0.0);
    a.set("fuse.pulled", 1.0);
    run(&mut a, 0.1);
    a.set("fuse.pulled", 0.0);
    run(&mut a, 0.1);
    assert_eq!(a.get("fuse.tripped"), Some(1.0), "pulling a fuse does nothing");
    a.set("new_fuse.on", 1.0);
    run(&mut a, 0.1);
    assert_eq!((a.get("fuse.tripped"), a.get("heater_bus.powered")), (Some(0.0), Some(1.0)));
}

#[test]
fn a_motor_draws_its_inrush_then_runs() {
    let mut a = aircraft(SWITCHING);
    a.set("main_on.on", 1.0);
    run(&mut a, 0.2);
    a.set("pump_on.on", 1.0);
    run(&mut a, 0.06);
    let start = get(&a, "pump.power");
    run(&mut a, 2.0);
    let running = get(&a, "pump.power");
    assert!(start > 2500.0 && (running - 500.0).abs() < 10.0, "inrush {start:.0} W, running {running:.0} W");
}

const CONVERTERS: &str = r#"
[[cockpit.switch]]
id = "fan_failed"

[[input.number]]
id = "demand"

[[electrical.bus]]
id = "ac"
nominal = "115 V"

[[electrical.bus]]
id = "dc"
nominal = "28 V"
powered_at = 0.5

[[electrical.bus]]
id = "bat_bus"
nominal = "24 V"

[[electrical.ext_power]]
id = "gpu"
bus = "ac"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
connected_when = "true"

[[electrical.tru]]
id = "tru"
input = "ac"
output = "dc"
voltage = "28 V"
rating = "1 kW"
cooled_when = "!fan_failed.on"

[[electrical.load]]
id = "dc_load"
bus = "dc"
power = "demand.value * 1 W"
type = "resistive"
min_voltage = "5 V"

[[electrical.charger]]
id = "charger"
input = "ac"
output = "bat_bus"
voltage = "28 V"
current = "20 A"

[[electrical.battery]]
id = "bat"
bus = "bat_bus"
voltage = "24 V"
capacity = "40 Ah"
charge = 0.3
charge_limit = "200 A"
"#;

#[test]
fn a_tru_folds_back_at_its_current_limit_and_overheats_without_cooling() {
    let mut a = aircraft(CONVERTERS);
    a.set("demand.value", 3000.0);
    run(&mut a, 3.0);
    let p = get(&a, "tru.power");
    assert!((1300.0..1700.0).contains(&p) && a.get("tru.limiting") == Some(1.0), "held near 150 %: {p:.0} W");
    assert!(get(&a, "dc.voltage") < 24.0);
    a.set("fan_failed.on", 1.0);
    run(&mut a, 300.0);
    assert_eq!((a.get("tru.overheat"), a.get("tru.active")), (Some(1.0), Some(0.0)), "{:.0} K", get(&a, "tru.temperature"));
}

#[test]
fn a_charger_charges_at_constant_current() {
    let mut a = aircraft(CONVERTERS);
    run(&mut a, 5.0);
    let i = get(&a, "bat.current");
    assert!((i + 20.0).abs() < 2.0, "charging at {i:.1} A");
    assert_eq!(a.get("charger.limiting"), Some(1.0));
    let q = get(&a, "bat.charge");
    run(&mut a, 60.0);
    assert!(get(&a, "bat.charge") > q, "charge rises");
    assert_eq!(a.get("ac.frequency"), Some(400.0));
    assert_eq!(a.get("dc.frequency"), Some(0.0));
}

