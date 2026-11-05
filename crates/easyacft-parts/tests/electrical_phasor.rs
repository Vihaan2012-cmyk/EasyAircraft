use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("phasor", src);
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

const AC: &str = r#"
[[cockpit.switch]]
id = "drive"
initial = true

[[cockpit.switch]]
id = "fault"

[[input.number]]
id = "spd"
initial = 1

[[input.number]]
id = "single"

[[input.number]]
id = "dc_demand"

[[electrical.bus]]
id = "ac"
nominal = "115 V"
short_when = "fault.on"
fault_type = "FAULT_TYPE"
fault_resistance = "0.002 ohm"

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
max_speed = 2.2

[[electrical.load]]
id = "galley"
bus = "ac"
power = "30 kW"
power_factor = 0.9

[[electrical.load]]
id = "oven"
bus = "ac"
power = "single.value * 1 kW"
phase = "a"
type = "resistive"

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.tru]]
id = "tru"
input = "ac"
output = "dc"
voltage = "29 V"
rating = "6 kW"
regulation = 0.08

[[electrical.load]]
id = "dc_load"
bus = "dc"
power = "dc_demand.value * 1 W"
type = "resistive"
min_voltage = "5 V"
"#;

fn ac(fault: &str) -> Aircraft {
    let mut a = aircraft(&AC.replace("FAULT_TYPE", fault));
    run(&mut a, 1.0);
    a
}

#[test]
fn a_balanced_bus_has_three_equal_phases_and_draws_reactive_power() {
    let a = ac("abc");
    for p in ["a", "b", "c"] {
        let v = get(&a, &format!("ac.voltage_{p}"));
        assert!((v - 115.0).abs() < 1.5, "phase {p} {v:.1} V");
    }
    assert!(get(&a, "ac.unbalance") < 0.005);
    let q = get(&a, "gen.reactive_power");
    assert!((q - 30e3 * (1.0 / 0.81 - 1.0f64).sqrt()).abs() < 2e3, "a 0.9 power factor galley draws {q:.0} var");
}

#[test]
fn a_single_phase_load_unbalances_the_bus_and_loads_one_phase() {
    let mut a = ac("abc");
    a.set("single.value", 25.0);
    run(&mut a, 2.0);
    let (ia, ib) = (get(&a, "gen.current_a"), get(&a, "gen.current_b"));
    assert!(ia > ib + 150.0, "phase a carries the oven: {ia:.0} A vs {ib:.0} A");
    assert!(get(&a, "ac.unbalance") > 0.003 && get(&a, "gen.negative_sequence") > 0.1, "unbalance {:.4}, I2 {:.3} pu", get(&a, "ac.unbalance"), get(&a, "gen.negative_sequence"));
}

#[test]
fn a_phase_to_ground_fault_collapses_its_phase_and_the_overcurrent_trip_clears_it() {
    let mut a = ac("a");
    a.set("fault.on", 1.0);
    run(&mut a, 0.1);
    let (va, vb) = (get(&a, "ac.voltage_a"), get(&a, "ac.voltage_b"));
    assert!(va < 40.0 && vb > 80.0, "faulted phase {va:.0} V, healthy phase {vb:.0} V");
    assert!(get(&a, "ac.fault_current") > 600.0, "{:.0} A into the fault", get(&a, "ac.fault_current"));
    run(&mut a, 1.0);
    assert_eq!(a.get("gen.fault"), Some(6.0), "overcurrent");
}

#[test]
fn a_phase_to_phase_fault_pulls_those_two_phases_together() {
    let mut a = ac("ab");
    a.set("fault.on", 1.0);
    run(&mut a, 0.1);
    let (va, vb, vc) = (get(&a, "ac.voltage_a"), get(&a, "ac.voltage_b"), get(&a, "ac.voltage_c"));
    assert!((va - vb).abs() < 10.0 && va < 95.0 && vc > 90.0, "a {va:.0}, b {vb:.0}, c {vc:.0} V");
}

#[test]
fn a_tru_droops_more_at_high_input_frequency() {
    let dc_at = |speed: f64| {
        let mut a = ac("abc");
        a.set("spd.value", speed);
        a.set("dc_demand.value", 5000.0);
        run(&mut a, 2.0);
        get(&a, "dc.voltage")
    };
    let (v400, v800) = (dc_at(1.0), dc_at(2.0));
    assert!(v400 < 29.0 && v800 < v400 - 0.2, "under the same load: {v400:.2} V at 400 Hz, {v800:.2} V at 800 Hz");
}

const MOTOR: &str = r#"
[[cockpit.switch]]
id = "on"

[[input.number]]
id = "scale"
initial = 1

[[cockpit.switch]]
id = "reversed"

[[electrical.bus]]
id = "ac"
nominal = "115 V"
powered_at = 0.4

[[electrical.ext_power]]
id = "gpu"
bus = "ac"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
connected_when = "true"
voltage_scale = "scale.value"
phase_reversed_when = "reversed.on"
monitor_time = "1 s"

[[electrical.load]]
id = "fan"
bus = "ac"
power = "3 kW"
type = "induction_motor"
on_when = "on.on"
start_time = "0.6 s"
"#;

#[test]
fn an_induction_motor_draws_its_inrush_accelerates_and_cooks_on_low_voltage() {
    let mut a = aircraft(MOTOR);
    run(&mut a, 0.5);
    a.set("on.on", 1.0);
    run(&mut a, 0.04);
    let inrush = get(&a, "fan.current");
    run(&mut a, 3.0);
    let running = get(&a, "fan.current");
    let speed = get(&a, "fan.speed");
    assert!(speed > 0.93 && speed < 1.0, "runs just under synchronous speed: {speed:.3}");
    assert!(inrush > 4.0 * running, "locked rotor {inrush:.1} A against running {running:.1} A");
    let p = get(&a, "fan.power");
    assert!((3000.0..4200.0).contains(&p), "takes its shaft power plus losses: {p:.0} W");
    a.set("scale.value", 0.55);
    run(&mut a, 3.0);
    let (slow, amps) = (get(&a, "fan.speed"), get(&a, "fan.current"));
    assert!(slow < 0.9 && amps > 1.5 * running, "on 55 % voltage it slips to {slow:.2} drawing {amps:.1} A");
    run(&mut a, 900.0);
    assert_eq!((a.get("fan.overheat"), a.get("fan.powered")), (Some(1.0), Some(0.0)), "its thermal switch opens at {:.0} K", get(&a, "fan.temperature"));
}

#[test]
fn the_external_power_monitor_rejects_a_bad_unit() {
    let mut a = aircraft(MOTOR);
    run(&mut a, 0.5);
    assert_eq!(a.get("gpu.available"), Some(0.0), "not before its monitor time");
    run(&mut a, 1.0);
    assert_eq!(a.get("gpu.available"), Some(1.0));
    a.set("reversed.on", 1.0);
    run(&mut a, 0.1);
    assert_eq!((a.get("gpu.available"), a.get("gpu.fault")), (Some(0.0), Some(9.0)), "phase sequence");
    a.set("reversed.on", 0.0);
    a.set("scale.value", 1.15);
    run(&mut a, 2.0);
    assert_eq!((a.get("gpu.available"), a.get("gpu.fault")), (Some(0.0), Some(1.0)), "overvoltage");
}

const SWITCHING: &str = r#"
[[input.number]]
id = "dc_scale"
initial = 1

[[cockpit.switch]]
id = "set"

[[cockpit.switch]]
id = "reset"

[[cockpit.switch]]
id = "cmd"
initial = true

[[input.number]]
id = "draw"

[[cockpit.switch]]
id = "dead_short"

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.ext_power]]
id = "dc_gpu"
bus = "dc"
voltage = "28 V"
rating = "20 kW"
connected_when = "true"
voltage_scale = "dc_scale.value"

[[electrical.bus]]
id = "far"
nominal = "28 V"

[[electrical.contactor]]
id = "k1"
between = ["dc", "far"]
coil = "dc"

[[electrical.bus]]
id = "latched_bus"
nominal = "28 V"

[[electrical.contactor]]
id = "k2"
between = ["dc", "latched_bus"]
coil = "dc"
latching = true
close_when = "set.on"
trip_when = "reset.on"

[[electrical.bus]]
id = "sspc_bus"
nominal = "28 V"
short_when = "dead_short.on"
fault_resistance = "0.001 ohm"

[[electrical.sspc]]
id = "ss"
between = ["dc", "sspc_bus"]
rating = "10 A"
on_when = "cmd.on"
trip_time = "0.5 s"
current_limit = 8
instant = 100

[[electrical.load]]
id = "heater"
bus = "sspc_bus"
power = "draw.value * 1 W"
type = "resistive"
"#;

#[test]
fn a_contactor_drops_out_when_its_coil_supply_sags_and_a_latched_one_holds() {
    let mut a = aircraft(SWITCHING);
    a.set("set.on", 1.0);
    run(&mut a, 0.2);
    a.set("set.on", 0.0);
    run(&mut a, 0.2);
    assert_eq!((a.get("k1.closed"), a.get("k2.closed")), (Some(1.0), Some(1.0)));
    a.set("dc_scale.value", 0.2);
    run(&mut a, 0.2);
    assert_eq!(a.get("k1.closed"), Some(0.0), "coil below its drop-out voltage");
    assert_eq!(a.get("k2.closed"), Some(1.0), "a latched contactor needs no coil power to stay shut");
    a.set("dc_scale.value", 1.0);
    run(&mut a, 0.2);
    assert_eq!(a.get("k1.closed"), Some(1.0), "pulls in again");
    a.set("reset.on", 1.0);
    run(&mut a, 0.2);
    assert_eq!(a.get("k2.closed"), Some(0.0), "the reset coil opens it");
}

#[test]
fn a_power_controller_trips_on_overload_and_resets_by_cycling_its_command() {
    let mut a = aircraft(SWITCHING);
    a.set("draw.value", 200.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("ss.tripped"), Some(0.0), "7 A is inside 10 A");
    a.set("draw.value", 700.0);
    run(&mut a, 2.0);
    assert_eq!((a.get("ss.tripped"), a.get("sspc_bus.powered")), (Some(1.0), Some(0.0)), "25 A trips its I²t curve");
    a.set("draw.value", 0.0);
    a.set("cmd.on", 0.0);
    run(&mut a, 0.1);
    a.set("cmd.on", 1.0);
    run(&mut a, 0.1);
    assert_eq!((a.get("ss.tripped"), a.get("sspc_bus.powered")), (Some(0.0), Some(1.0)), "cycling the command resets it");
}

#[test]
fn a_power_controller_limits_a_dead_short_until_it_trips() {
    let mut a = aircraft(SWITCHING);
    run(&mut a, 0.2);
    a.set("dead_short.on", 1.0);
    run(&mut a, 0.02);
    let i = get(&a, "ss.current");
    assert!((60.0..100.0).contains(&i), "held near 8 x 10 A: {i:.0} A");
    run(&mut a, 0.3);
    assert_eq!(a.get("ss.tripped"), Some(1.0));
}

const FEEDER: &str = r#"
[[cockpit.switch]]
id = "chafe"

[[input.number]]
id = "draw"

[[electrical.bus]]
id = "src"
nominal = "28 V"

[[electrical.ext_power]]
id = "dc_gpu"
bus = "src"
voltage = "28 V"
rating = "40 kW"
connected_when = "true"

[[electrical.bus]]
id = "mid"
nominal = "28 V"

[[electrical.breaker]]
id = "cb"
between = ["src", "mid"]
rating = "150 A"

[[electrical.bus]]
id = "end"
nominal = "28 V"

[[electrical.feeder]]
id = "run"
between = ["mid", "end"]
length = "30 m"
gauge = "4"
chafed_when = "chafe.on"

[[electrical.load]]
id = "load"
bus = "end"
power = "draw.value * 1 W"
type = "resistive"
min_voltage = "5 V"
"#;

#[test]
fn a_feeder_drops_voltage_warms_with_current_and_a_chafe_trips_its_breaker() {
    let mut a = aircraft(FEEDER);
    a.set("draw.value", 2800.0);
    run(&mut a, 1.0);
    let drop = get(&a, "run.voltage_drop");
    assert!((1.5..4.5).contains(&drop), "100 A down 30 m of 4 AWG: {drop:.2} V");
    let t0 = get(&a, "run.temperature");
    run(&mut a, 600.0);
    let t1 = get(&a, "run.temperature");
    assert!(t1 > t0 + 15.0, "the conductor warms: {t0:.0} to {t1:.0} K");
    a.set("chafe.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("cb.tripped"), Some(1.0), "the chafe to structure trips the breaker upstream");
}

#[test]
fn a_transformer_steps_up_and_droops_with_load() {
    let mut a = aircraft(
        r#"
[[input.number]]
id = "draw"

[[electrical.bus]]
id = "ac115"
nominal = "115 V"

[[electrical.ext_power]]
id = "gpu"
bus = "ac115"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
connected_when = "true"

[[electrical.bus]]
id = "ac230"
nominal = "230 V"

[[electrical.transformer]]
id = "atu"
between = ["ac115", "ac230"]
ratio = 0.5
rating = "60 kVA"

[[electrical.load]]
id = "load"
bus = "ac230"
power = "draw.value * 1 kW"
type = "resistive"
"#,
    );
    run(&mut a, 0.5);
    let open = get(&a, "ac230.voltage");
    assert!((open - 230.0).abs() < 3.0, "no load: {open:.1} V");
    a.set("draw.value", 50.0);
    run(&mut a, 0.5);
    let loaded = get(&a, "ac230.voltage");
    assert!(loaded < open - 1.0 && loaded > 0.9 * 230.0, "loaded: {loaded:.1} V");
    let p = get(&a, "atu.power");
    assert!((40e3..55e3).contains(&p), "carries the load: {p:.0} W");
}

const BATTERY: &str = r#"
[[input.number]]
id = "draw"

[[cockpit.switch]]
id = "charger"

[[electrical.bus]]
id = "dc"
nominal = "24 V"

[[electrical.battery]]
id = "bat"
bus = "dc"
voltage = "24 V"
capacity = "40 Ah"
charge = 0.8

[[electrical.load]]
id = "load"
bus = "dc"
power = "draw.value * 1 W"
type = "resistive"
min_voltage = "1 V"

[[electrical.bus]]
id = "li"
nominal = "28 V"

[[electrical.battery]]
id = "lith"
bus = "li"
voltage = "28.8 V"
capacity = "30 Ah"
chemistry = "lithium"
charge = 0.95
charge_limit = "500 A"

[[electrical.ext_power]]
id = "overcharger"
bus = "li"
voltage = "36 V"
rating = "10 kW"
connected_when = "charger.on"
"#;

#[test]
fn a_battery_sags_under_load_and_recovers_after_it() {
    let mut a = aircraft(BATTERY);
    run(&mut a, 1.0);
    let rest = get(&a, "bat.voltage");
    a.set("draw.value", 3000.0);
    run(&mut a, 0.05);
    let first = get(&a, "bat.voltage");
    run(&mut a, 60.0);
    let held = get(&a, "bat.voltage");
    assert!(held < first - 0.2, "polarisation sags it further: {first:.2} then {held:.2} V");
    a.set("draw.value", 0.0);
    run(&mut a, 0.1);
    let quick = get(&a, "bat.voltage");
    run(&mut a, 120.0);
    let later = get(&a, "bat.voltage");
    assert!(later > quick + 0.1 && later < rest, "recovers over a minute or two: {quick:.2} then {later:.2} V, rested {rest:.2} V");
}

#[test]
fn a_lithium_battery_management_system_opens_on_overcharge() {
    let mut a = aircraft(BATTERY);
    run(&mut a, 1.0);
    assert_eq!(a.get("lith.bms_open"), Some(0.0));
    a.set("charger.on", 1.0);
    run(&mut a, 5.0);
    assert_eq!(a.get("lith.bms_open"), Some(1.0), "cells at {:.2} V", get(&a, "lith.cell_voltage"));
    assert_eq!(a.get("lith.current"), Some(0.0));
}

const PARALLEL: &str = r#"
[[cockpit.switch]]
id = "tie"

[[input.number]]
id = "spd2"
initial = 1

[[electrical.bus]]
id = "ac1"
nominal = "115 V"

[[electrical.bus]]
id = "ac2"
nominal = "115 V"

[[electrical.generator]]
id = "g1"
bus = "ac1"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"

[[electrical.generator]]
id = "g2"
bus = "ac2"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
speed = "spd2.value"
variable_frequency = "VF"
max_speed = 1.5

[[electrical.contactor]]
id = "btc"
between = ["ac1", "ac2"]
close_when = "tie.on"
sync_check = true

[[electrical.load]]
id = "l1"
bus = "ac1"
power = "60 kW"
"#;

#[test]
fn constant_frequency_generators_parallel_in_sync_and_share_the_load() {
    let mut a = aircraft(&PARALLEL.replace("\"VF\"", "false"));
    run(&mut a, 1.0);
    a.set("tie.on", 1.0);
    run(&mut a, 15.0);
    assert_eq!(a.get("btc.closed"), Some(1.0), "the two drives slip slowly into phase and the tie closes");
    run(&mut a, 5.0);
    let (p1, p2) = (get(&a, "g1.load"), get(&a, "g2.load"));
    assert!(p2 > 15e3 && (p1 - p2).abs() < 0.35 * (p1 + p2), "both carry the 60 kW: {p1:.0} W and {p2:.0} W");
    assert!((get(&a, "ac2.frequency") - 400.0).abs() < 2.0 && a.get("g1.tripped") == Some(0.0) && a.get("g2.tripped") == Some(0.0));
}

#[test]
fn the_sync_check_keeps_the_tie_open_between_generators_at_different_frequencies() {
    let mut a = aircraft(&PARALLEL.replace("\"VF\"", "true"));
    a.set("spd2.value", 1.05);
    run(&mut a, 1.0);
    a.set("tie.on", 1.0);
    run(&mut a, 3.0);
    assert_eq!(a.get("btc.closed"), Some(0.0), "420 Hz will not parallel with 400 Hz");
    assert_eq!(a.get("btc.in_sync"), Some(0.0));
}

const ZONE: &str = r#"
[[cockpit.switch]]
id = "drive"
initial = true

[[cockpit.switch]]
id = "feeder_fault"

[[cockpit.switch]]
id = "reset"

[[electrical.bus]]
id = "gen_out"
nominal = "115 V"
short_when = "feeder_fault.on"

[[electrical.bus]]
id = "ac"
nominal = "115 V"

[[electrical.generator]]
id = "gen"
bus = "gen_out"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
driven_when = "drive.on"
zone = "glc"
reset_when = "reset.on"

[[electrical.contactor]]
id = "glc"
between = ["gen_out", "ac"]
close_when = "gen.ready"

[[electrical.load]]
id = "load"
bus = "ac"
power = "40 kW"
"#;

#[test]
fn a_feeder_fault_inside_the_zone_trips_the_differential_and_locks_out() {
    let mut a = aircraft(ZONE);
    run(&mut a, 2.0);
    assert_eq!((a.get("gen.ready"), a.get("glc.closed")), (Some(1.0), Some(1.0)));
    run(&mut a, 2.0);
    assert_eq!(a.get("gen.tripped"), Some(0.0), "load through the zone is not a fault");
    a.set("feeder_fault.on", 1.0);
    run(&mut a, 0.2);
    assert_eq!(a.get("gen.fault"), Some(7.0), "differential, well before overcurrent");
    a.set("feeder_fault.on", 0.0);
    a.set("drive.on", 0.0);
    run(&mut a, 0.5);
    a.set("drive.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("gen.tripped"), Some(1.0), "a differential trip stays locked out through a drive cycle");
    a.set("reset.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!((a.get("gen.tripped"), a.get("gen.ready")), (Some(0.0), Some(1.0)), "only a reset clears it");
}

#[test]
fn a_dc_starter_generator_cranks_then_comes_on_line() {
    let mut a = aircraft(
        r#"
[[cockpit.switch]]
id = "start"

[[input.number]]
id = "n2"

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.battery]]
id = "bat"
bus = "dc"
voltage = "24 V"
capacity = "40 Ah"
resistance = "0.01 ohm"

[[electrical.generator]]
id = "sg"
bus = "dc"
voltage = "28.5 V"
rating = "12 kW"
speed = "n2.value"
start_when = "start.on"
start_current = "800 A"
min_speed = 0.5
"#,
    );
    a.set("start.on", 1.0);
    a.set("n2.value", 0.05);
    run(&mut a, 0.5);
    let i = get(&a, "sg.starter_current");
    assert!((500.0..820.0).contains(&i) && get(&a, "sg.torque") < 0.0, "cranking at {i:.0} A, torque {:.1} N m", get(&a, "sg.torque"));
    assert!(get(&a, "bat.current") > 400.0, "off the battery");
    a.set("n2.value", 0.3);
    run(&mut a, 0.5);
    assert!(get(&a, "sg.starter_current") < i, "back-emf cuts the current as it speeds up");
    a.set("start.on", 0.0);
    a.set("n2.value", 0.9);
    run(&mut a, 2.0);
    assert_eq!(a.get("sg.ready"), Some(1.0));
    assert!(get(&a, "bat.current") < 0.0 && (get(&a, "dc.voltage") - 28.5).abs() < 0.6, "it charges the battery at {:.1} V", get(&a, "dc.voltage"));
}

#[test]
fn electronics_ride_through_a_short_transfer_and_reboot_after_a_long_one() {
    let mut a = aircraft(
        r#"
[[cockpit.switch]]
id = "gpu_on"
initial = true

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.ext_power]]
id = "gpu"
bus = "dc"
voltage = "28 V"
rating = "5 kW"
connected_when = "gpu_on.on"

[[electrical.load]]
id = "display"
bus = "dc"
power = "150 W"
type = "electronic"
min_voltage = "18 V"
holdup = "0.05 s"
boot_time = "2 s"
"#,
    );
    run(&mut a, 1.0);
    assert_eq!(a.get("display.powered"), Some(1.0));
    a.set("gpu_on.on", 0.0);
    run(&mut a, 0.02);
    a.set("gpu_on.on", 1.0);
    run(&mut a, 0.1);
    assert_eq!((a.get("display.powered"), a.get("display.resetting")), (Some(1.0), Some(0.0)), "a 20 ms break is inside its hold-up");
    a.set("gpu_on.on", 0.0);
    run(&mut a, 0.3);
    a.set("gpu_on.on", 1.0);
    run(&mut a, 0.5);
    assert_eq!((a.get("display.powered"), a.get("display.resetting")), (Some(0.0), Some(1.0)), "a 300 ms break makes it reboot");
    run(&mut a, 2.5);
    assert_eq!(a.get("display.powered"), Some(1.0));
}

#[test]
fn auto_phased_loads_spread_across_the_phases() {
    let mut src = String::from(
        r#"
[[electrical.bus]]
id = "ac"
nominal = "115 V"

[[electrical.ext_power]]
id = "gpu"
bus = "ac"
voltage = "115 V"
frequency = "400 Hz"
rating = "90 kVA"
connected_when = "true"
"#,
    );
    for k in 0..6 {
        src.push_str(&format!("\n[[electrical.load]]\nid = \"l{k}\"\nbus = \"ac\"\npower = \"2 kW\"\nphase = \"auto\"\n"));
    }
    let mut a = aircraft(&src);
    run(&mut a, 0.5);
    let phases: Vec<f64> = (0..6).map(|k| get(&a, &format!("l{k}.phase"))).collect();
    assert_eq!(phases, vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
    assert!(get(&a, "ac.unbalance") < 0.002, "balanced by spreading");
}

#[test]
fn nothing_blows_up_when_everything_is_thrown_at_it() {
    let mut a = ac("ab");
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % 10_000) as f64 / 10_000.0
    };
    for _ in 0..400 {
        a.set("fault.on", (rnd() < 0.3) as u8 as f64);
        a.set("drive.on", (rnd() < 0.8) as u8 as f64);
        a.set("spd.value", 2.5 * rnd());
        a.set("single.value", 200.0 * rnd());
        a.set("dc_demand.value", 20_000.0 * rnd());
        a.step(0.02 + 0.2 * rnd());
        for k in ["ac.voltage", "ac.voltage_a", "gen.current", "gen.load", "dc.voltage", "tru.power", "gen.field", "gen.rotor_angle"] {
            assert!(get(&a, k).is_finite(), "{k} went non-finite");
        }
    }
}
