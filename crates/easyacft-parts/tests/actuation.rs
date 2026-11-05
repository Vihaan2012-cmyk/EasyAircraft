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
cooling_temperature = "oil_t.value * 1 K"
heat_exchanger = 20000

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
oscillation_when = "ofc.on"

[[actuator.linear]]
id = "ail_b"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"
active_when = "false"

[[actuator.linear]]
id = "aa_1"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"

[[actuator.linear]]
id = "aa_2"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"

[[actuator.linear]]
id = "ab_1"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"

[[actuator.linear]]
id = "ab_2"
circuit = "green"
force = "60 kN"
rate = "0.08 m/s"

[[flight_control.surface]]
id = "outboard"
actuators = ["aa_1", "aa_2"]
command = "ail_cmd.value"
air = "air"
force_equalization = true

[[flight_control.surface]]
id = "inboard"
actuators = ["ab_1", "ab_2"]
command = "ail_cmd.value"
air = "air"

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
line_length = "45 m"
line_bore = "8 mm"
active_when = "!act_off.on"

[[flight_control.surface]]
id = "rudder"
actuators = ["rud_a"]
command = "rud_cmd.value"
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
bogie = "mlg_bogie"
load = "mlg_load.value * 1 N"
blowdown_when = "blow.on"
crank = "crank.value"
release_turns = 3
limit_speed = "250 kt"
air = "air"

[[input.number]]
id = "crank"

[[input.number]]
id = "ema_lost"

[[cockpit.switch]]
id = "bearing"

[[cockpit.switch]]
id = "nose_down"
initial = true

[[brakes.wheel]]
id = "w2"
electric = true
pedal = "pedal.value"
actuators_failed = "ema_lost.value"
bearing_failed_when = "bearing.on"
air = "air"

[[actuator.linear]]
id = "nlg_jack"
circuit = "green"
control = "selector"
force = "90 kN"
rate = "0.15 m/s"
rod = 0.4

[[landing_gear.door]]
id = "nlg_door"
linked = true

[[landing_gear.leg]]
id = "nlg"
actuator = "nlg_jack"
doors = ["nlg_door"]
down_when = "nose_down.on"
retracts = "forward"
mass = "250 kg"
cg = "0.9 m"
air = "air"

[[cockpit.switch]]
id = "blow"

[[input.number]]
id = "spoiler_cmd"

[[actuator.linear]]
id = "spl_a"
circuit = "green"
force = "40 kN"
rate = "0.06 m/s"

[[flight_control.surface]]
id = "spoiler"
actuators = ["spl_a"]
command = "spoiler_cmd.value"
air = "air"
area = 2
chord = "0.8 m"
min = "0 deg"
max = "50 deg"
deflects = "up"
ch0 = -0.1
free_play = "0.5 mm"
turbulence = "0.5"

[[actuator.linear]]
id = "trimmer"
circuit = "green"
force = "60 kN"
rate = "0.05 m/s"

[[landing_gear.bogie]]
id = "mlg_bogie"
actuator = "trimmer"
on_ground_when = "mlg.compressed"
jam_when = "bogie_jam.on"

[[cockpit.switch]]
id = "bogie_jam"

[[cockpit.switch]]
id = "ofc"

[[input.number]]
id = "mlg_load"

[[input.number]]
id = "water"

[[input.number]]
id = "rud_cmd"

[[input.number]]
id = "oil_t"
initial = 323.15

[[brakes.wheel]]
id = "w1"
circuit = "green"
alternate_circuit = "green"
pedal = "pedal.value"
parking_when = "parking.on"
runway_friction = "runway.value"
line_burst_when = "burst.on"
water_depth = "water.value * 1 m"
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
centring_when = "cams.on"
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
vfe = "200 kt"

[[actuator.linear]]
id = "spare"
circuit = "green"
force = "10 kN"
rate = "0.05 m/s"
stroke = "0.1 m"
command = "spare_cmd.value"
lvdt_1_fault = "lvdt_fault.value"

[[input.number]]
id = "spare_cmd"
initial = 0.9

[[input.number]]
id = "lvdt_fault"

[[input.number]]
id = "bus"
initial = 540

[[input.number]]
id = "eha_cmd"
initial = 0.5

[[actuator.linear]]
id = "eha"
control = "eha"
force = "80 kN"
rate = "0.06 m/s"
stroke = "0.08 m"
command = "eha_cmd.value"
voltage = "bus.value * 1 V"
load = "30 kN"

[[cockpit.switch]]
id = "burst"

[[cockpit.switch]]
id = "cams"

[[input.number]]
id = "pilot"

[[input.number]]
id = "trim_sw"

[[flight_control.feel]]
id = "pitch_feel"
pilot_force = "pilot.value * 1 N"
trim = "trim_sw.value"
air = "air"
q_feel = true
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
    assert!(get(&a, "nws.angle").abs() < 1.0 * DEG, "back to {:.2} deg, error {:.2}", get(&a, "nws.angle") / DEG, get(&a, "nws.error") / DEG);
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

#[test]
fn a_detached_transducer_core_runs_the_actuator_to_its_stop() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("spare_cmd.value", 0.5);
    run(&mut a, 3.0);
    assert!((get(&a, "spare.position") - 0.5).abs() < 0.02);
    a.set("lvdt_fault.value", 3.0);
    a.set("spare_cmd.value", 0.6);
    run(&mut a, 4.0);
    let pos = get(&a, "spare.position");
    assert!(pos > 0.97, "with its feedback frozen it drives to the stop: {pos:.2}");
    assert!(get(&a, "spare.lvdt_disagree") != 0.0, "channels {:.3} {:.3}", get(&a, "spare.lvdt_1"), get(&a, "spare.lvdt_2"));
    a.set("lvdt_fault.value", 1.0);
    run(&mut a, 4.0);
    assert!((get(&a, "spare.position") - 0.6).abs() < 0.03, "open coil: falls back on channel 2, at {:.2}", get(&a, "spare.position"));
}

#[test]
fn eha_moves_on_its_own_pump_and_slows_on_a_sagging_bus() {
    let mut a = rig();
    ground(&mut a);
    a.set("pump_on.on", 0.0);
    run(&mut a, 2.0);
    let start = get(&a, "eha.position");
    a.set("eha_cmd.value", 0.9);
    run(&mut a, 0.5);
    let moved = get(&a, "eha.position") - start;
    assert!(moved > 0.15, "EHA moved {moved:.3} of stroke in 0.5 s with no hydraulics");
    assert!(get(&a, "eha.electric_power") > 100.0 && get(&a, "eha.current") > 0.1, "{:.0} W {:.1} A", get(&a, "eha.electric_power"), get(&a, "eha.current"));
    assert!(get(&a, "eha.circuit_flow") == 0.0);
    run(&mut a, 3.0);
    assert!((get(&a, "eha.position") - 0.9).abs() < 0.02, "at {:.3}", get(&a, "eha.position"));
    a.set("eha_cmd.value", 0.2);
    a.set("bus.value", 200.0);
    run(&mut a, 0.5);
    let slow = 0.9 - get(&a, "eha.position");
    assert!(slow < 0.7 * moved, "a sagging bus: {slow:.3} against {moved:.3}");
    a.set("bus.value", 0.0);
    let held = get(&a, "eha.position");
    run(&mut a, 2.0);
    assert!(get(&a, "eha.active") == 0.0 && get(&a, "eha.position") < held - 0.01, "unpowered it gives way to its load in damping: {held:.2} -> {:.2}", get(&a, "eha.position"));
}

#[test]
fn a_burst_brake_line_trips_its_fuse_before_the_circuit_drains() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    let fluid0 = get(&a, "green.fluid");
    a.set("pedal.value", 0.5);
    a.set("burst.on", 1.0);
    run(&mut a, 5.0);
    assert!(get(&a, "w1.hydraulic_fuse") != 0.0, "fuse open");
    let lost = fluid0 - get(&a, "green.fluid");
    assert!(lost > 0.05e-3 && lost < 2e-3, "lost {:.2} L", lost * 1e3);
    assert!(get(&a, "green.pressure") > 2500.0 * PSI);
}

#[test]
fn parked_brake_pressure_climbs_as_hot_brakes_soak_the_housing() {
    let mut a = rig();
    ground(&mut a);
    a.set("air.groundspeed", 50.0);
    a.set("air.airspeed", 50.0);
    run(&mut a, 2.0);
    a.set("pedal.value", 1.0);
    let mut v: f64 = 50.0;
    for _ in 0..2000 {
        v = (v - get(&a, "w1.force") / 15_000.0 * 0.02).max(0.0);
        a.set("air.groundspeed", v);
        a.set("air.airspeed", v);
        a.step(0.02);
    }
    a.set("parking.on", 1.0);
    run(&mut a, 3.0);
    a.set("pedal.value", 0.0);
    let p0 = get(&a, "w1.pressure");
    run(&mut a, 600.0);
    let p1 = get(&a, "w1.pressure");
    assert!(p1 > p0 + 20.0 * PSI, "trapped {:.0} -> {:.0} psi, housing {:.0} K", p0 / PSI, p1 / PSI, get(&a, "w1.stack_temperature"));
    assert!(p1 < 1.2 * 3000.0 * PSI, "thermal relief caps it");
}

#[test]
fn feel_force_follows_dynamic_pressure_and_trim_moves_the_neutral() {
    let mut a = rig();
    flying(&mut a, 120.0);
    a.set("pilot.value", 150.0);
    run(&mut a, 3.0);
    let slow = get(&a, "pitch_feel.position");
    flying(&mut a, 300.0);
    run(&mut a, 3.0);
    let fast = get(&a, "pitch_feel.position");
    assert!(slow > 1.8 * fast && fast > 0.0, "same push: {slow:.3} slow, {fast:.3} fast");
    a.set("pilot.value", 0.0);
    a.set("trim_sw.value", 1.0);
    run(&mut a, 4.0);
    a.set("trim_sw.value", 0.0);
    run(&mut a, 3.0);
    let rest = get(&a, "pitch_feel.position");
    assert!((rest - get(&a, "pitch_feel.neutral")).abs() < 0.05 && rest > 0.15, "hands off it rests at the trimmed neutral: {rest:.3}");
}

#[test]
fn centring_cams_straighten_the_nosewheel_after_lift_off() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("steer.value", 30.0 * DEG);
    run(&mut a, 3.0);
    assert!(get(&a, "nws.angle") > 25.0 * DEG);
    a.set("air.on_ground", 0.0);
    a.set("cams.on", 1.0);
    run(&mut a, 3.0);
    assert!(get(&a, "nws.angle").abs() < 1.0 * DEG, "centred to {:.1} deg", get(&a, "nws.angle") / DEG);
}

#[test]
fn surface_transducers_follow_the_surface() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("ail_cmd.value", -12.0 * DEG);
    run(&mut a, 2.0);
    let t = get(&a, "aileron.angle");
    assert!((get(&a, "aileron.sensor_1") - t).abs() < 0.3 * DEG && (get(&a, "aileron.sensor_2") - t).abs() < 0.3 * DEG);
    assert!(get(&a, "aileron.sensor_disagree") == 0.0);
    assert!(get(&a, "flaps.fppu") >= -0.01 && get(&a, "ths.sensor_1").is_finite());
}

#[test]
fn a_bogie_must_trail_before_its_leg_retracts() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    assert!(get(&a, "mlg_bogie.level") != 0.0, "on the ground it lies level: {:.1} deg", get(&a, "mlg_bogie.angle") / DEG);
    flying(&mut a, 160.0);
    run(&mut a, 4.0);
    assert!(get(&a, "mlg_bogie.trailed") != 0.0, "in the air it trails: {:.1} deg", get(&a, "mlg_bogie.angle") / DEG);
    let mut b = rig();
    ground(&mut b);
    run(&mut b, 3.0);
    b.set("bogie_jam.on", 1.0);
    flying(&mut b, 160.0);
    run(&mut b, 3.0);
    b.set("gear_down.on", 0.0);
    run(&mut b, 15.0);
    assert!(get(&b, "mlg.down_locked") != 0.0 && get(&b, "mlg.retract_inhibited") != 0.0, "a level bogie holds the gear down");
}

#[test]
fn a_firm_arrival_is_logged_as_a_hard_landing() {
    let mut a = rig();
    flying(&mut a, 140.0);
    run(&mut a, 3.0);
    assert!(get(&a, "mlg.strut_stroke") < 0.01);
    a.set("air.on_ground", 1.0);
    a.set("mlg_load.value", 300e3);
    run(&mut a, 2.0);
    assert!(get(&a, "mlg.hard_landing") == 0.0, "gentle: {:.2} m/s", get(&a, "mlg.strut_rate"));
    a.set("mlg_load.value", 0.0);
    a.set("air.on_ground", 0.0);
    run(&mut a, 3.0);
    a.set("air.on_ground", 1.0);
    a.set("mlg_load.value", 2.5e6);
    run(&mut a, 0.3);
    a.set("mlg_load.value", 300e3);
    run(&mut a, 2.0);
    assert!(get(&a, "mlg.hard_landing") != 0.0, "peak {:.0} kN", get(&a, "mlg.peak_load") / 1e3);
}

#[test]
fn tyres_hydroplane_on_standing_water_at_speed() {
    let mut a = rig();
    ground(&mut a);
    a.set("water.value", 0.006);
    a.set("air.groundspeed", 75.0);
    a.set("air.airspeed", 75.0);
    run(&mut a, 2.0);
    assert!(get(&a, "w1.hydroplaning") != 0.0);
    a.set("pedal.value", 0.6);
    run(&mut a, 1.5);
    assert!(get(&a, "w1.locked") != 0.0, "wheel at {:.1} m/s on the water", get(&a, "w1.wheel_speed"));
    a.set("pedal.value", 0.0);
    a.set("air.groundspeed", 35.0);
    a.set("air.airspeed", 35.0);
    run(&mut a, 2.0);
    assert!(get(&a, "w1.hydroplaning") == 0.0);
}

#[test]
fn an_oscillatory_servo_failure_shakes_the_surface() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    let calm = get(&a, "aileron.oscillation");
    a.set("ofc.on", 1.0);
    run(&mut a, 3.0);
    let shaking = get(&a, "aileron.oscillation");
    assert!(shaking > 0.05 && shaking > 10.0 * calm.max(1e-4), "oscillation {calm:.4} -> {shaking:.4} rad/s");
}

#[test]
fn a_cold_soaked_tail_line_slows_the_rudder() {
    let time_to = |oil: f64| {
        let mut a = rig();
        ground(&mut a);
        a.set("oil_t.value", oil);
        run(&mut a, 60.0);
        a.set("rud_cmd.value", 20.0 * DEG);
        let mut t = 0.0;
        while t < 10.0 && get(&a, "rudder.angle") < 18.0 * DEG {
            a.step(0.02);
            t += 0.02;
        }
        (t, get(&a, "rud_a.line_drop"), get(&a, "green.temperature"))
    };
    let (warm, warm_drop, wt) = time_to(323.15);
    let (cold, cold_drop, ct) = time_to(238.15);
    assert!(cold > 1.3 * warm, "rudder to 18 deg: {warm:.2} s at {wt:.0} K, {cold:.2} s at {ct:.0} K; line drop {:.0} vs {:.0} psi", warm_drop / PSI, cold_drop / PSI);
}

#[test]
fn worn_rod_ends_let_the_surface_rattle_in_its_free_play() {
    let mut a = rig();
    flying(&mut a, 280.0);
    run(&mut a, 2.0);
    a.set("spoiler_cmd.value", 20.0 * DEG);
    run(&mut a, 3.0);
    let ang = get(&a, "spoiler.angle") / DEG;
    assert!((ang - 20.0).abs() < 1.5, "spoiler at {ang:.1} deg");
    let mut most: f64 = 0.0;
    for _ in 0..200 {
        a.step(0.02);
        most = most.max(get(&a, "spoiler.slack").abs());
    }
    assert!(most > 0.2e-3 && most < 3e-3, "slack {:.2} mm", most * 1e3);
    assert!(get(&a, "spl_a.position").is_finite());
}

#[test]
fn a_nitrogen_bottle_blows_the_gear_down_without_hydraulics() {
    let mut a = rig();
    flying(&mut a, 180.0);
    run(&mut a, 2.0);
    a.set("gear_down.on", 0.0);
    run(&mut a, 30.0);
    assert!(get(&a, "mlg.up_locked") != 0.0);
    a.set("pump_on.on", 0.0);
    run(&mut a, 60.0);
    let full = get(&a, "mlg.bottle_pressure");
    a.set("blow.on", 1.0);
    a.set("gear_down.on", 1.0);
    let mut t = 0.0;
    while t < 20.0 && get(&a, "mlg.down_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(get(&a, "mlg.down_locked") != 0.0, "blown down in {t:.1} s");
    assert!(get(&a, "mlg.bottle_pressure") < 0.8 * full, "bottle {:.0} -> {:.0} psi", full / PSI, get(&a, "mlg.bottle_pressure") / PSI);
}

#[test]
fn touchdown_spin_up_scrubs_tread_off() {
    let mut a = rig();
    flying(&mut a, 140.0);
    run(&mut a, 30.0);
    assert!(get(&a, "w1.wheel_speed") < 5.0, "spun down in the air: {:.1}", get(&a, "w1.wheel_speed"));
    let before = get(&a, "w1.tread");
    a.set("air.on_ground", 1.0);
    a.set("air.altitude", 0.0);
    run(&mut a, 2.0);
    let after = get(&a, "w1.tread");
    assert!((get(&a, "w1.wheel_speed") - 140.0 * KT).abs() < 2.0, "spun up to {:.1}", get(&a, "w1.wheel_speed"));
    let worn = before - after;
    assert!(worn > 1e-4 && worn < 1e-2, "one touchdown wore {:.4} of the tread", worn);
}

#[test]
fn eha_windings_heat_with_duty() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 2.0);
    for k in 0..60 {
        a.set("eha_cmd.value", if k % 2 == 0 { 0.1 } else { 0.9 });
        run(&mut a, 2.0);
    }
    let wind = get(&a, "eha.winding_temperature");
    let body = get(&a, "eha.temperature");
    assert!(wind > body + 1.0 && body > 288.5, "winding {wind:.1} K over body {body:.1} K");
}

#[test]
fn the_gravity_crank_isolates_then_frees_the_doors_then_the_leg() {
    let mut a = rig();
    flying(&mut a, 180.0);
    run(&mut a, 2.0);
    a.set("gear_down.on", 0.0);
    run(&mut a, 30.0);
    a.set("pump_on.on", 0.0);
    run(&mut a, 60.0);
    a.set("gear_down.on", 1.0);
    a.set("crank.value", 1.5);
    run(&mut a, 4.0);
    assert!(get(&a, "mlg.up_locked") != 0.0, "one and a half turns: still up");
    a.set("crank.value", 2.2);
    run(&mut a, 10.0);
    assert!(get(&a, "mlg_door.locked") == 0.0 && get(&a, "mlg_door.position") > 0.05 && get(&a, "mlg.up_locked") != 0.0, "doors unlocked and hanging, leg held: door {:.2}", get(&a, "mlg_door.position"));
    a.set("crank.value", 3.0);
    let mut t = 0.0;
    while t < 40.0 && get(&a, "mlg.down_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(get(&a, "mlg.down_locked") != 0.0, "three turns: down after {t:.1} s");
}

#[test]
fn gear_and_flaps_out_too_fast_are_overstressed() {
    let mut a = rig();
    flying(&mut a, 240.0);
    run(&mut a, 3.0);
    assert!(get(&a, "mlg.overspeed") == 0.0);
    flying(&mut a, 330.0);
    run(&mut a, 1.0);
    assert!(get(&a, "mlg.overspeed") != 0.0 && get(&a, "mlg.overstressed") == 0.0);
    run(&mut a, 3.0);
    assert!(get(&a, "mlg.overstressed") != 0.0, "VLE exceeded by a quarter for seconds");
    let mut b = rig();
    flying(&mut b, 180.0);
    run(&mut b, 2.0);
    b.set("flap_cmd.value", 0.3);
    run(&mut b, 15.0);
    assert!(get(&b, "flaps.vfe_exceeded") == 0.0);
    flying(&mut b, 260.0);
    run(&mut b, 5.0);
    assert!(get(&b, "flaps.vfe_exceeded") != 0.0 && get(&b, "flaps.overstressed") != 0.0);
}

#[test]
fn a_cold_soaked_strut_sits_lower() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 10.0);
    let hot = get(&a, "mlg.strut_stroke");
    let mut c = rig();
    ground(&mut c);
    // A cold-soaked aircraft: the standard atmosphere at 9 km gives -43 C.
    c.set("air.altitude", 9000.0);
    run(&mut c, 10.0);
    let cold = get(&c, "mlg.strut_stroke");
    assert!(cold > hot + 0.005, "strut {:.3} m at 15 C, {:.3} m at {:.0} K", hot, cold, get(&c, "air.temperature"));
}

#[test]
fn electric_brakes_clamp_without_hydraulics_and_lose_torque_with_actuators() {
    let mut a = rig();
    ground(&mut a);
    a.set("pump_on.on", 0.0);
    a.set("air.groundspeed", 30.0);
    a.set("air.airspeed", 30.0);
    run(&mut a, 2.0);
    a.set("pedal.value", 0.5);
    run(&mut a, 1.0);
    let full = get(&a, "w2.torque").abs();
    assert!(full > 1000.0 && get(&a, "w2.electric_power") > 100.0, "{full:.0} N m at {:.0} W", get(&a, "w2.electric_power"));
    assert!(get(&a, "w2.circuit_flow") == 0.0);
    a.set("ema_lost.value", 2.0);
    run(&mut a, 1.0);
    let half = get(&a, "w2.torque").abs();
    assert!((half / full - 0.5).abs() < 0.15, "two of four actuators: {half:.0} of {full:.0} N m");
}

#[test]
fn a_linked_door_rides_on_its_leg() {
    let mut a = rig();
    flying(&mut a, 170.0);
    run(&mut a, 2.0);
    assert!(get(&a, "nlg_door.open") != 0.0, "down: door open on the leg");
    a.set("nose_down.on", 0.0);
    let mut t = 0.0;
    while t < 30.0 && get(&a, "nlg.up_locked") == 0.0 {
        run(&mut a, 0.1);
        t += 0.1;
        let leg = get(&a, "nlg.position");
        if leg > 0.25 {
            assert!(get(&a, "nlg_door.position") > 0.97, "leg at {leg:.2}, door {:.2}", get(&a, "nlg_door.position"));
        }
    }
    run(&mut a, 0.1);
    assert!(get(&a, "nlg.up_locked") != 0.0 && get(&a, "nlg_door.closed") != 0.0, "up in {t:.1} s, door {:.2}", get(&a, "nlg_door.position"));
}

#[test]
fn a_failing_wheel_bearing_cooks_the_hub() {
    let rim = |bad: bool| {
        let mut a = rig();
        ground(&mut a);
        a.set("air.groundspeed", 15.0);
        a.set("air.airspeed", 15.0);
        a.set("bearing.on", bad as u8 as f64);
        run(&mut a, 300.0);
        get(&a, "w2.rim_temperature")
    };
    let good = rim(false);
    let bad = rim(true);
    assert!(bad > good + 40.0, "rim {good:.0} K healthy, {bad:.0} K with the bearing gone");
}

#[test]
fn active_active_actuators_with_equalization_stop_fighting() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 3.0);
    a.set("ail_cmd.value", 5.0 * DEG);
    run(&mut a, 30.0);
    let plain = get(&a, "inboard.force_fight");
    let equalized = get(&a, "outboard.force_fight");
    assert!(plain > 1000.0, "transducer offsets alone set two active actuators fighting: {plain:.0} N");
    assert!(equalized < 0.3 * plain, "equalization takes the fight out: {plain:.0} -> {equalized:.0} N");
}

#[test]
fn tyre_pressure_display_lags_by_its_sample_period() {
    let mut a = rig();
    ground(&mut a);
    run(&mut a, 10.0);
    let shown = get(&a, "w1.tpis_pressure");
    assert!((shown - get(&a, "w1.tyre_pressure")).abs() < 3.0 * PSI, "{:.0} psi shown", shown / PSI);
    assert!((get(&a, "w1.wear_pin") - 0.025).abs() < 1e-3);
}

#[test]
#[ignore]
fn time_rig() {
    let mut a = rig();
    flying(&mut a, 250.0);
    a.start_profile();
    let t = std::time::Instant::now();
    for k in 0..3000 {
        a.set("ail_cmd.value", 10.0 * DEG * ((k as f64) * 0.02).sin());
        a.step(1.0 / 60.0);
    }
    let el = t.elapsed().as_secs_f64();
    for d in a.profile() {
        println!("{} {:.3} ms/frame", d.name, d.seconds / 3000.0 * 1e3);
    }
    println!("total {:.3} ms/frame", el / 3000.0 * 1e3);
}
