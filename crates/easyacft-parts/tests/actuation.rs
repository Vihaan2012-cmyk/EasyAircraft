use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const DEG: f64 = std::f64::consts::PI / 180.0;
const KT: f64 = 1852.0 / 3600.0;
const PSI: f64 = 6894.757;

const RIG: &str = r#"
[[environment.air]]
id = "air"
standard = true

[[cockpit.switch]]
id = "pump_on"
initial = true

[[cockpit.switch]]
id = "gear_down"
initial = true

[[cockpit.switch]]
id = "free_fall"

[[cockpit.switch]]
id = "act_off"

[[cockpit.switch]]
id = "m2_off"

[[cockpit.switch]]
id = "shaft_break"

[[cockpit.switch]]
id = "parking"

[[input.number]]
id = "ail_cmd"

[[input.number]]
id = "elev_cmd"

[[input.number]]
id = "pedal"

[[input.number]]
id = "steer"

[[input.number]]
id = "ths_cmd"

[[input.number]]
id = "flap_cmd"

[[input.number]]
id = "runway"
initial = 0.8

[[hydraulic.circuit]]
id = "green"
nominal = "3000 psi"
fluid = "60 L"

[[hydraulic.pump]]
id = "edp"
circuit = "green"
pressure = "3000 psi"
flow = "160 L/min"
running_when = "pump_on.on"

[[hydraulic.accumulator]]
id = "acc"
circuit = "green"
volume = "3 L"
precharge = "1500 psi"

[[actuator.linear]]
id = "ail_a"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"
active_when = "!act_off.on"

[[actuator.linear]]
id = "ail_b"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"
active_when = "false"

[[flight_control.surface]]
id = "aileron"
actuators = ["ail_a", "ail_b"]
command = "ail_cmd.value"
air = "air"
area = 3
chord = "0.7 m"

[[actuator.linear]]
id = "elev_a"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"
disconnect_when = "act_off.on"

[[flight_control.surface]]
id = "elevator"
actuators = ["elev_a"]
command = "elev_cmd.value"
air = "air"
area = 3
chord = "0.7 m"
orientation = "horizontal"
flutter_speed = "250 kt"
turbulence = "0.3"

[[actuator.linear]]
id = "rud_a"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"
active_when = "!act_off.on"

[[flight_control.surface]]
id = "rudder"
actuators = ["rud_a"]
air = "air"
area = 3
chord = "0.7 m"
orientation = "vertical"
flutter_speed = "250 kt"
turbulence = "0.3"

[[actuator.linear]]
id = "mlg_jack"
circuit = "green"
control = "selector"
force = "200 kN"
rate = "0.15 m/s"
rod = 0.4

[[actuator.linear]]
id = "door_jack"
circuit = "green"
control = "selector"
force = "20 kN"
rate = "0.2 m/s"
rod = 0.3

[[landing_gear.door]]
id = "mlg_door"
actuator = "door_jack"
air = "air"

[[landing_gear.leg]]
id = "mlg"
actuator = "mlg_jack"
doors = ["mlg_door"]
down_when = "gear_down.on"
free_fall_when = "free_fall.on"
air = "air"

[[brakes.wheel]]
id = "w1"
circuit = "green"
alternate_circuit = "green"
pedal = "pedal.value"
parking_when = "parking.on"
runway_friction = "runway.value"
air = "air"

[[actuator.linear]]
id = "nws_a"
circuit = "green"
force = "150 kN"
rate = "0.1 m/s"

[[actuator.linear]]
id = "nws_b"
circuit = "green"
force = "150 kN"
rate = "0.1 m/s"

[[steering.unit]]
id = "nws"
actuators = ["nws_a", "nws_b"]
command = "steer.value"
air = "air"

[[actuator.motor]]
id = "ths_m1"
circuit = "green"

[[actuator.motor]]
id = "ths_m2"
circuit = "green"
enabled_when = "!m2_off.on"

[[flight_control.stabilizer]]
id = "ths"
motors = ["ths_m1", "ths_m2"]
command = "ths_cmd.value"
air = "air"

[[actuator.motor]]
id = "flap_m1"
circuit = "green"

[[actuator.motor]]
id = "flap_m2"
circuit = "green"
enabled_when = "!m2_off.on"

[[flight_control.high_lift]]
id = "flaps"
motors = ["flap_m1", "flap_m2"]
command = "flap_cmd.value"
air = "air"
shaft_break_left_when = "shaft_break.on"

[[actuator.linear]]
id = "spare"
circuit = "green"
force = "10 kN"
rate = "0.05 m/s"
stroke = "0.1 m"
command = "0.9"
"#;

fn rig() -> Aircraft {
    let (m, mut d) = load_str("rig", RIG);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn get(a: &Aircraft, k: &str) -> f64 {
    a.get(k).unwrap_or_else(|| panic!("no {k}"))
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s / 0.02).round() as usize {
        a.step(0.02);
    }
}

fn ground(a: &mut Aircraft) {
    a.set("air.on_ground", 1.0);
    a.set("air.altitude", 0.0);
}

fn flying(a: &mut Aircraft, kt: f64) {
    a.set("air.on_ground", 0.0);
    a.set("air.altitude", 1000.0);
    a.set("air.airspeed", kt * KT);
    a.set("air.groundspeed", kt * KT);
}

#[test]
fn surface_follows_its_command_and_droops_with_no_pressure() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    assert!(get(&a, "green.pressure") > 2500.0 * PSI, "{} psi", get(&a, "green.pressure") / PSI);
    a.set("ail_cmd.value", 10.0 * DEG);
    run(&mut a, 1.5);
    let ang = get(&a, "aileron.angle") / DEG;
    assert!((ang - 10.0).abs() < 0.5, "aileron at {ang:.2} deg");
    assert!(get(&a, "ail_a.active") != 0.0 && get(&a, "ail_b.active") == 0.0);
    a.set("ail_cmd.value", 0.0);
    a.set("pump_on.on", 0.0);
    run(&mut a, 60.0);
    let droop = get(&a, "aileron.angle") / DEG;
    assert!(droop > 3.0 && get(&a, "aileron.drooped") != 0.0, "drooped to {droop:.2} deg, pressure {:.0} psi", get(&a, "green.pressure") / PSI);
}

#[test]
fn high_dynamic_pressure_blows_the_surface_down() {
    let mut a = rig();
    flying(&mut a, 380.0);
    run(&mut a, 3.0);
    a.set("ail_cmd.value", 25.0 * DEG);
    run(&mut a, 3.0);
    let ang = get(&a, "aileron.angle") / DEG;
    assert!(ang < 22.0 && ang > 5.0, "aileron reached {ang:.1} deg against the airload");
    assert!(get(&a, "aileron.blowdown") != 0.0, "hinge {:.0} N m", get(&a, "aileron.hinge_moment"));
    flying(&mut a, 150.0);
    run(&mut a, 3.0);
    let low = get(&a, "aileron.angle") / DEG;
    assert!((low - 25.0).abs() < 1.0, "slow: {low:.1} deg");
}

#[test]
fn a_disconnected_surface_flutters_past_its_flutter_speed_and_a_damped_one_does_not() {
    let mut a = rig();
    flying(&mut a, 330.0);
    run(&mut a, 2.0);
    a.set("act_off.on", 1.0);
    run(&mut a, 20.0);
    let osc = get(&a, "elevator.oscillation");
    assert!(get(&a, "elevator.flutter") != 0.0, "elevator rate swing {osc:.3} rad/s");
    let rud = get(&a, "rudder.oscillation");
    assert!(get(&a, "rudder.flutter") == 0.0, "damped rudder swings {rud:.3} rad/s");
}

#[test]
fn gear_retracts_and_extends_in_sequence() {
    let mut a = rig();
    flying(&mut a, 180.0);
    run(&mut a, 3.0);
    assert!(get(&a, "mlg.down_locked") != 0.0 && get(&a, "mlg_door.closed") != 0.0);
    let level0 = get(&a, "green.fluid");
    a.set("gear_down.on", 0.0);
    let mut doors_first = false;
    let mut t = 0.0;
    while t < 30.0 && get(&a, "mlg.up_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
        if get(&a, "mlg_door.open") != 0.0 && get(&a, "mlg.position") > 0.95 {
            doors_first = true;
        }
        assert!(get(&a, "mlg.position") > 0.97 || get(&a, "mlg.up_locked") != 0.0 || get(&a, "mlg_door.position") > 0.9, "leg moved at {:.2} with the door at {:.2}", get(&a, "mlg.position"), get(&a, "mlg_door.position"));
    }
    assert!(doors_first && get(&a, "mlg.up_locked") != 0.0, "up after {t:.1} s");
    assert!(t > 4.0 && t < 20.0, "retraction took {t:.1} s");
    run(&mut a, 8.0);
    assert!(get(&a, "mlg_door.closed") != 0.0, "door at {:.2}", get(&a, "mlg_door.position"));
    let level_up = get(&a, "green.fluid");
    assert!((level_up - level0).abs() > 0.5e-3, "reservoir {:.2} -> {:.2} L", level0 * 1e3, level_up * 1e3);
    a.set("gear_down.on", 1.0);
    let mut t = 0.0;
    while t < 30.0 && get(&a, "mlg.down_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(get(&a, "mlg.down_locked") != 0.0 && t < 20.0, "extension took {t:.1} s");
    run(&mut a, 8.0);
    assert!(get(&a, "mlg_door.closed") != 0.0);
}

#[test]
fn gear_free_falls_and_locks_without_hydraulics() {
    let mut a = rig();
    flying(&mut a, 180.0);
    run(&mut a, 2.0);
    a.set("gear_down.on", 0.0);
    run(&mut a, 30.0);
    assert!(get(&a, "mlg.up_locked") != 0.0);
    a.set("pump_on.on", 0.0);
    run(&mut a, 60.0);
    a.set("gear_down.on", 1.0);
    run(&mut a, 5.0);
    assert!(get(&a, "mlg.up_locked") != 0.0, "stays up with no pressure");
    a.set("free_fall.on", 1.0);
    let mut t = 0.0;
    while t < 40.0 && get(&a, "mlg.down_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(get(&a, "mlg.down_locked") != 0.0, "free fall reached {:.2}", get(&a, "mlg.position"));
    assert!(t > 3.0, "fell in {t:.1} s, too fast for fluid to leave the jack");
}

#[test]
fn brakes_heat_up_stopping_and_lock_on_ice() {
    let mut a = rig();
    ground(&mut a);
    a.set("air.groundspeed", 60.0);
    a.set("air.airspeed", 60.0);
    run(&mut a, 2.0);
    assert!((get(&a, "w1.wheel_speed") - 60.0).abs() < 1.0, "spun up {:.1}", get(&a, "w1.wheel_speed"));
    a.set("pedal.value", 1.0);
    run(&mut a, 1.0);
    let p = get(&a, "w1.pressure") / PSI;
    assert!(p > 2500.0, "brake pressure {p:.0} psi");
    let mut v: f64 = 60.0;
    for _ in 0..1500 {
        let f = get(&a, "w1.force");
        v = (v - f / 20_000.0 * 0.02).max(0.0);
        a.set("air.groundspeed", v);
        a.set("air.airspeed", v);
        a.step(0.02);
        if v <= 0.0 {
            break;
        }
    }
    assert!(v < 1.0, "stopped from 60 m/s, at {v:.1}");
    let e = get(&a, "w1.energy");
    assert!(e > 2e7, "absorbed {:.1} MJ", e / 1e6);
    assert!(get(&a, "w1.stack_temperature") > 400.0, "stack {:.0} K", get(&a, "w1.stack_temperature"));
    assert!(get(&a, "w1.locked") == 0.0 && get(&a, "w1.burst") == 0.0);
    run(&mut a, 60.0);
    assert!(get(&a, "w1.temperature") > 380.0, "thermocouple {:.0} K", get(&a, "w1.temperature"));

    let mut b = rig();
    ground(&mut b);
    b.set("runway.value", 0.05);
    b.set("air.groundspeed", 50.0);
    b.set("air.airspeed", 50.0);
    run(&mut b, 2.0);
    b.set("pedal.value", 1.0);
    run(&mut b, 2.0);
    assert!(get(&b, "w1.locked") != 0.0 && get(&b, "w1.skidding") != 0.0, "on ice: wheel {:.1} m/s", get(&b, "w1.wheel_speed"));
    run(&mut b, 10.0);
    assert!(get(&b, "w1.burst") != 0.0, "flat spot {:.2}", get(&b, "w1.flat_spot"));
}

#[test]
fn parking_brake_holds_after_the_pump_stops() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("parking.on", 1.0);
    run(&mut a, 2.0);
    let p0 = get(&a, "w1.pressure");
    assert!(p0 > 2500.0 * PSI, "{:.0} psi", p0 / PSI);
    a.set("pump_on.on", 0.0);
    run(&mut a, 120.0);
    let p1 = get(&a, "w1.pressure");
    assert!(p1 > 0.8 * p0, "parked {:.0} -> {:.0} psi with the circuit at {:.0}", p0 / PSI, p1 / PSI, get(&a, "green.pressure") / PSI);
}

#[test]
fn steering_turns_against_scrub_and_castors_when_towed() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("steer.value", 40.0 * DEG);
    run(&mut a, 4.0);
    let ang = get(&a, "nws.angle") / DEG;
    assert!((ang - 40.0).abs() < 1.5, "standstill steer {ang:.1} deg, scrub {:.0} N m", get(&a, "nws.scrub"));
    a.set("steer.value", 0.0);
    run(&mut a, 4.0);
    assert!(get(&a, "nws.angle").abs() < 1.0 * DEG);
}

#[test]
fn stabilizer_trims_at_half_rate_on_one_motor() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("ths_cmd.value", 2.0 * DEG);
    run(&mut a, 2.0);
    let both = get(&a, "ths.angle") / DEG;
    assert!(both > 0.7 && both < 1.2, "two motors trimmed {both:.2} deg in 2 s");
    run(&mut a, 6.0);
    assert!((get(&a, "ths.angle") / DEG - 2.0).abs() < 0.1);
    a.set("m2_off.on", 1.0);
    a.set("ths_cmd.value", 0.0);
    run(&mut a, 2.0);
    let one = 2.0 - get(&a, "ths.angle") / DEG;
    assert!(one > 0.35 && one < 0.65, "one motor trimmed {one:.2} deg in 2 s");
    assert!(get(&a, "ths_m2.braked") != 0.0);
}

#[test]
fn flaps_run_out_on_two_motors_and_a_shaft_break_shows_asymmetry() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("flap_cmd.value", 1.0);
    run(&mut a, 20.0);
    let half = get(&a, "flaps.position");
    assert!(half > 0.4 && half < 0.6, "20 s in: {half:.2}");
    a.set("shaft_break.on", 1.0);
    run(&mut a, 10.0);
    assert!(get(&a, "flaps.asymmetry").abs() > 0.15, "asymmetry {:.2}", get(&a, "flaps.asymmetry"));
}

#[test]
fn actuators_draw_from_the_circuit_while_they_move() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("ail_cmd.value", 20.0 * DEG);
    a.step(0.02);
    a.step(0.02);
    let q = get(&a, "ail_a.circuit_flow");
    assert!(q > 5e-3 / 60.0, "aileron draws {:.1} L/min", q * 60e3);
    run(&mut a, 2.0);
    let idle = get(&a, "ail_a.circuit_flow");
    assert!(idle < 1.5e-3 / 60.0, "at rest it leaks {:.2} L/min", idle * 60e3);
    assert!((get(&a, "spare.position") - 0.9).abs() < 0.02, "free actuator at {:.2}", get(&a, "spare.position"));
}

#[test]
fn abuse_never_blows_up() {
    let mut a = rig();
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let switches = ["pump_on.on", "gear_down.on", "free_fall.on", "act_off.on", "m2_off.on", "parking.on"];
    for step in 0..3000 {
        if step % 50 == 0 {
            for s in switches {
                a.set(s, (rnd() > 0.5) as u8 as f64);
            }
            a.set("ail_cmd.value", (rnd() - 0.5) * 60.0 * DEG);
            a.set("elev_cmd.value", (rnd() - 0.5) * 60.0 * DEG);
            a.set("steer.value", (rnd() - 0.5) * 3.0);
            a.set("ths_cmd.value", (rnd() - 0.5) * 30.0 * DEG);
            a.set("flap_cmd.value", rnd() * 1.4 - 0.2);
            a.set("pedal.value", rnd());
            a.set("runway.value", rnd());
            let v = rnd() * 250.0;
            a.set("air.airspeed", v);
            a.set("air.groundspeed", v * rnd());
            a.set("air.on_ground", (rnd() > 0.5) as u8 as f64);
        }
        let dt = [0.005, 0.02, 0.05, 0.1, 0.25][step % 5];
        a.step(dt);
    }
    let bad: Vec<String> = a.names().filter(|n| a.get(n).is_some_and(|v| !v.is_finite())).map(|s| s.to_owned()).collect();
    assert!(bad.is_empty(), "non-finite: {bad:?}");
}
