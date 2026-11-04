use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const AIRFRAME: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "lever"

[[cockpit.switch]]
id = "fans"

[[cockpit.switch]]
id = "towing"

[[cockpit.selector]]
id = "o2_mode"
positions = 3

[[hydraulic.circuit]]
id = "green"
nominal = "3000 psi"
fluid = "40 L"

[[hydraulic.pump]]
id = "edp"
circuit = "green"
pressure = "3000 psi"
flow = "150 L/min"

[[gear.leg]]
id = "main"
circuit = "green"
air = "air"
down_when = "lever.on"
time = "8 s"
door_time = "2 s"
airload = "150 m/s"
swept_volume = "12 L"
retract_inhibit_when = "air.on_ground"

[[gear.brake]]
id = "brakes"
pedal = "1"
max_pressure = "3000 psi"
air = "air"
heat_sink = "400 kg"
max_torque = "40000 N*m"
tyre_radius = "0.6 m"
wheels = 4
fan_when = "fans.on"

[[gear.tyre]]
id = "tyre"
pressure = "218 psi"
brake = "brakes"
air = "air"

[[gear.steering]]
id = "nws"
circuit = "green"
tiller = "1"
air = "air"
towing_when = "towing.on"

[[cabin.pressurization]]
id = "cabin"
air = "air"
volume = "1600 m3"
inflow = "4 kg/s"
max_differential = "8.9 psi"
cabin_ceiling = "8000 ft"
breach_area = "if(lever.on, 0.0, 0.0)"
cabin_heat = "60 kW"

[[oxygen.crew]]
id = "crew_o2"
pressure = "1850 psi"
capacity = "3200 L"
masks_on = "2"
mode = "o2_mode.position"
cabin = "cabin"

[[oxygen.passenger]]
id = "pax_o2"
deploy_when = "cabin.altitude > 14000 ft"
masks = 100
pulled = "0.5"
"#;

fn aircraft(src: &str) -> Aircraft {
    let (model, mut d) = load_str("airframe", src);
    match Aircraft::from_model(model, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s * 50.0) as usize {
        a.step(0.02);
    }
}

fn fl(a: &mut Aircraft, pressure: f64, temperature: f64) {
    a.set("air.pressure", pressure);
    a.set("air.temperature", temperature);
    a.set("air.on_ground", 0.0);
}

#[test]
fn the_cabin_holds_its_differential_at_cruise_and_climbs_past_14000_ft_after_a_breach() {
    let mut a = aircraft(AIRFRAME);
    fl(&mut a, 23_842.0, 218.8);
    run(&mut a, 1500.0);
    let ft = a.get("cabin.altitude").unwrap() / 0.3048;
    let diff = a.get("cabin.differential").unwrap() / 6894.757;
    // 8.65 psi over FL350 is about 5 500 ft of cabin: the controller holds the lowest cabin its differential allows.
    assert!((5000.0..6500.0).contains(&ft), "cabin {ft:.0} ft at FL350");
    assert!((8.3..8.9).contains(&diff), "differential {diff:.2} psi");
    assert_eq!(a.get("cabin.safety_valve"), Some(0.0), "the safety valves stay shut while the controller holds");
    let rate = a.get("cabin.vertical_speed").unwrap().abs();
    assert!(rate < 0.2, "steady: {rate:.2} m/s");
    let t = a.get("cabin.temperature").unwrap() - 273.15;
    assert!((5.0..35.0).contains(&t), "cabin {t:.0} C with 22 C pack air and the wall losing heat to -54 C");
}

#[test]
fn a_hole_decompresses_the_cabin_and_drops_the_masks() {
    let src = AIRFRAME.replace(r#"breach_area = "if(lever.on, 0.0, 0.0)""#, r#"breach_area = "if(fans.on, 0.5, 0.0)""#);
    let mut a = aircraft(&src);
    fl(&mut a, 23_842.0, 218.8);
    run(&mut a, 1500.0);
    a.set("fans.on", 1.0);
    run(&mut a, 30.0);
    let ft = a.get("cabin.altitude").unwrap() / 0.3048;
    assert!(ft > 25_000.0, "a 0.5 m2 hole takes the cabin most of the way to the aircraft in 30 s: {ft:.0} ft");
    assert_eq!(a.get("cabin.excess_altitude"), Some(1.0));
    assert_eq!(a.get("pax_o2.deployed"), Some(1.0));
    let flow = a.get("pax_o2.flow").unwrap() * 6e4;
    assert!((150.0..210.0).contains(&flow), "50 generators near 4 L/min each: {flow:.0} L/min");
    assert_eq!(a.get("pax_o2.active"), Some(50.0));
    run(&mut a, 15.0 * 60.0);
    assert_eq!(a.get("pax_o2.exhausted"), Some(1.0), "generators burn their 15 minutes and stop");
}

#[test]
fn crew_oxygen_dilutes_low_and_runs_pure_high() {
    let mut a = aircraft(AIRFRAME);
    run(&mut a, 10.0);
    let sea = a.get("crew_o2.flow").unwrap();
    assert!(sea < 1e-6, "at sea level the diluter gives air: {sea}");
    fl(&mut a, 23_842.0, 218.8);
    a.set("cabin.failed", 1.0);
    run(&mut a, 1500.0);
    let normal = a.get("crew_o2.flow").unwrap();
    a.set("o2_mode.position", 1.0);
    run(&mut a, 1.0);
    let pure = a.get("crew_o2.flow").unwrap();
    assert!(pure >= normal && pure > 0.0, "100 % never draws less than diluted: {normal} vs {pure}");
    let hours = a.get("crew_o2.endurance").unwrap() / 3600.0;
    assert!(hours > 0.5 && hours < 20.0, "endurance {hours:.1} h for two crew");
}

#[test]
fn the_gear_opens_its_doors_first_and_airload_slows_retraction() {
    let mut a = aircraft(AIRFRAME);
    run(&mut a, 2.0);
    a.set("air.on_ground", 0.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("main.position"), Some(1.0), "doors open before the leg moves");
    assert!(a.get("main.doors").unwrap() > 0.3);
    run(&mut a, 20.0);
    assert_eq!(a.get("main.up_locked"), Some(1.0));
    assert_eq!(a.get("main.doors_closed"), Some(1.0), "doors close behind it");
    a.set("lever.on", 1.0);
    let mut t = 0.0;
    while a.get("main.down_locked") != Some(1.0) && t < 30.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(t > 8.0 && t < 12.0, "extension with doors at rest air: {t:.1} s");
    a.set("air.airspeed", 100.0);
    a.set("lever.on", 0.0);
    let mut t = 0.0;
    while a.get("main.up_locked") != Some(1.0) && t < 60.0 {
        run(&mut a, 0.1);
        t += 0.1;
    }
    assert!(t > 8.0 * 1.3, "retraction against 100 m/s airload is slower than its 8 s: {t:.1} s");
    a.set("air.on_ground", 1.0);
    a.set("lever.on", 1.0);
    run(&mut a, 20.0);
    a.set("lever.on", 0.0);
    run(&mut a, 5.0);
    assert_eq!(a.get("main.down_locked"), Some(1.0), "no retraction with weight on wheels");
}

#[test]
fn brakes_heat_from_the_energy_they_absorb_and_tyres_follow() {
    let mut a = aircraft(AIRFRAME);
    run(&mut a, 2.0);
    a.set("air.groundspeed", 70.0);
    run(&mut a, 10.0);
    let e = a.get("brakes.energy").unwrap() / 1e6;
    // 4 x 40 kN*m at 70/0.6 rad/s for 10 s is 187 MJ at nominal friction.
    assert!((150.0..260.0).contains(&e), "absorbed {e:.0} MJ");
    let t = a.get("brakes.temperature").unwrap() - 273.15;
    assert!(t > 300.0, "400 kg of carbon after {e:.0} MJ: {t:.0} C");
    assert_eq!(a.get("brakes.hot"), Some(1.0));
    let f = a.get("brakes.force").unwrap();
    assert!(f > 100_000.0, "retarding force {f:.0} N");
    a.set("air.groundspeed", 0.0);
    run(&mut a, 600.0);
    let p = a.get("tyre.pressure").unwrap() / 6894.757;
    assert!(p > 218.0, "a tyre warmed by its brake reads above its cold pressure: {p:.0} psi");
    let still = a.get("brakes.temperature").unwrap();
    a.set("fans.on", 1.0);
    run(&mut a, 120.0);
    let fanned = a.get("brakes.temperature").unwrap();
    let mut b = aircraft(AIRFRAME);
    b.set("air.groundspeed", 70.0);
    run(&mut b, 10.0);
    b.set("air.groundspeed", 0.0);
    run(&mut b, 720.0);
    assert!(fanned < b.get("brakes.temperature").unwrap() && fanned < still, "fans cool it faster");
    let mut c = aircraft(AIRFRAME);
    c.set("air.groundspeed", 70.0);
    run(&mut c, 25.0);
    assert_eq!(c.get("brakes.fuse_melted"), Some(1.0), "a 470 MJ stop melts the fuse plugs");
    c.set("air.groundspeed", 0.0);
    run(&mut c, 60.0);
    assert_eq!(c.get("tyre.deflated"), Some(1.0));
}

#[test]
fn steering_authority_falls_with_speed_and_the_wheel_castors_when_towed() {
    let mut a = aircraft(AIRFRAME);
    run(&mut a, 10.0);
    let slow = a.get("nws.angle").unwrap();
    assert!((slow - 70.0).abs() < 0.5, "full tiller at a standstill: {slow:.1}");
    a.set("air.groundspeed", 40.0);
    run(&mut a, 10.0);
    let fast = a.get("nws.angle").unwrap();
    assert!(fast < 10.0, "above 70 kt only the pedals steer: {fast:.1}");
    a.set("air.groundspeed", 5.0);
    run(&mut a, 10.0);
    a.set("towing.on", 1.0);
    run(&mut a, 20.0);
    assert_eq!(a.get("nws.castoring"), Some(1.0));
    assert!(a.get("nws.angle").unwrap().abs() < 60.0, "trails back as it rolls");
}

#[test]
fn snapshots_carry_the_new_state() {
    let mut a = aircraft(AIRFRAME);
    a.set("air.groundspeed", 70.0);
    run(&mut a, 5.0);
    let snap = a.snapshot();
    let t = a.get("brakes.temperature").unwrap();
    run(&mut a, 5.0);
    a.restore(&snap);
    run(&mut a, 0.02);
    assert!((a.get("brakes.temperature").unwrap() - t).abs() < 5.0);
}
