use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const AIRCRAFT: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "eng1"

[[cockpit.switch]]
id = "acmp"

[[cockpit.switch]]
id = "gear_lever"
initial = true

[[cockpit.switch]]
id = "xfeed"

[[cockpit.switch]]
id = "pump_r_sw"
initial = true

[[hydraulic.circuit]]
id = "green"
nominal = "3000 psi"
fluid = "20 L"

[[hydraulic.circuit]]
id = "yellow"
nominal = "3000 psi"
fluid = "15 L"

[[hydraulic.pump]]
id = "edp1"
circuit = "green"
pressure = "3000 psi"
flow = "140 L/min"
running_when = "eng1.on"

[[hydraulic.pump]]
id = "emp"
circuit = "yellow"
pressure = "3000 psi"
flow = "30 L/min"
running_when = "acmp.on"

[[hydraulic.accumulator]]
id = "acc"
circuit = "green"
volume = "4 L"
precharge = "1000 psi"

[[hydraulic.consumer]]
id = "spoilers"
circuit = "green"
flow = "40 L/min"
moving_when = "true"

[[hydraulic.ptu]]
id = "ptu"
between = ["green", "yellow"]
flow = "60 L/min"

[[gear.leg]]
id = "main"
circuit = "green"
down_when = "gear_lever.on"
time = "8 s"

[[fuel.tank]]
id = "left"
capacity = "2000 kg"
quantity = "1500 kg"
arm = "10 m"

[[fuel.tank]]
id = "right"
capacity = "2000 kg"
quantity = "1500 kg"
arm = "10 m"

[[fuel.line]]
id = "feed_l"

[[fuel.line]]
id = "feed_r"

[[fuel.pump]]
id = "pump_l"
tank = "left"
outlet = "feed_l"
flow = "2 kg/s"

[[fuel.pump]]
id = "pump_r"
tank = "right"
outlet = "feed_r"
flow = "2 kg/s"
running_when = "pump_r_sw.on"

[[fuel.valve]]
id = "crossfeed"
between = ["feed_l", "feed_r"]
open_when = "xfeed.on"

[[fuel.feed]]
id = "eng1_feed"
line = "feed_l"
demand = "if(eng1.on, 0.8 kg/s, 0 kg/s)"

[[fuel.feed]]
id = "eng2_feed"
line = "feed_r"
demand = "if(eng1.on, 0.8 kg/s, 0 kg/s)"

[[pneumatic.duct]]
id = "bleed1"

[[pneumatic.duct]]
id = "pack_inlet"

[[pneumatic.source]]
id = "hp_port"
duct = "bleed1"
pressure = "80 psi"
temperature = "500 degC"
flow = "3 kg/s"
available_when = "eng1.on"

[[pneumatic.valve]]
id = "prv"
between = ["bleed1", "pack_inlet"]
regulate = "44 psi"
cool_to = "200 degC"

[[pneumatic.consumer]]
id = "pack1"
duct = "pack_inlet"
flow = "1.2 kg/s"

[[cabin.pressurization]]
id = "cabin"
air = "air"
volume = "800 m3"
inflow = "pack1.flow"

[[ice.surface]]
id = "wing"
air = "air"
heated_when = "eng1.on"
build_time = "100 s"

[[oxygen.passenger]]
id = "masks"
deploy_when = "cabin.altitude > 14000 ft"

[[weight.balance]]
id = "wb"
stations = ["left", "right"]
"#;

fn aircraft() -> Aircraft {
    let (m, mut d) = load_str("systems", AIRCRAFT);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s * 30.0) as usize {
        a.step(1.0 / 30.0);
    }
}

#[test]
fn hydraulics_pressurise_and_the_ptu_helps_the_other_side() {
    let mut a = aircraft();
    run(&mut a, 2.0);
    assert!(a.get("green.pressure").unwrap() < 1e5, "no pump running");
    a.set("eng1.on", 1.0);
    run(&mut a, 3.0);
    let g = a.get("green.pressure").unwrap() / 6894.757;
    assert!((2400.0..3000.0).contains(&g), "green {g:.0} psi");
    assert_eq!(a.get("green.available"), Some(1.0));
    let y = a.get("yellow.pressure").unwrap() / 6894.757;
    assert!(y > 1000.0, "the PTU drives yellow from green: {y:.0} psi");
    assert_eq!(a.get("spoilers.powered"), Some(1.0));
    a.set("gear_lever.on", 0.0);
    run(&mut a, 4.0);
    let pos = a.get("main.position").unwrap();
    assert!(pos > 0.3 && pos < 0.7, "half way after 4 of 8 s: {pos:.2}");
    run(&mut a, 5.0);
    assert_eq!(a.get("main.up_locked"), Some(1.0));
}

#[test]
fn fuel_flows_to_the_engines_and_crossfeeds() {
    let mut a = aircraft();
    a.set("eng1.on", 1.0);
    run(&mut a, 10.0);
    assert_eq!(a.get("eng1_feed.starved"), Some(0.0));
    let used = 1500.0 - a.get("left.quantity").unwrap();
    assert!((used - 8.0).abs() < 1.0, "0.8 kg/s for 10 s from the left tank: {used:.2}");
    assert!((a.get("wb.mass").unwrap() - (3000.0 - 16.0)).abs() < 2.0);
    a.set("pump_r_sw.on", 0.0);
    run(&mut a, 2.0);
    assert_eq!(a.get("eng2_feed.starved"), Some(1.0), "no pump and no crossfeed");
    a.set("xfeed.on", 1.0);
    run(&mut a, 2.0);
    assert_eq!(a.get("eng2_feed.starved"), Some(0.0), "the left pump feeds both through the crossfeed");
    let before = a.get("right.quantity").unwrap();
    run(&mut a, 5.0);
    assert!((a.get("right.quantity").unwrap() - before).abs() < 1e-6, "the right tank no longer feeds");
}

#[test]
fn bleed_regulates_and_cools_and_the_cabin_pressurises() {
    let mut a = aircraft();
    a.set("eng1.on", 1.0);
    run(&mut a, 20.0);
    let p = a.get("pack_inlet.pressure").unwrap() / 6894.757;
    assert!((35.0..50.0).contains(&p), "regulated near 44 psi: {p:.1}");
    let t = a.get("pack_inlet.temperature").unwrap() - 273.15;
    assert!(t < 210.0 && t > 150.0, "precooled to about 200 C: {t:.0}");
    assert_eq!(a.get("pack1.supplied"), Some(1.0));
    a.set("air.pressure", 23_842.0);
    a.set("air.temperature", 218.8);
    a.set("air.on_ground", 0.0);
    run(&mut a, 400.0);
    let cabin_ft = a.get("cabin.altitude").unwrap() / 0.3048;
    let diff = a.get("cabin.differential").unwrap() / 6894.757;
    assert!(cabin_ft < 9000.0, "cabin {cabin_ft:.0} ft at FL350");
    assert!(diff > 6.0 && diff < 8.8, "differential {diff:.1} psi");
    assert_eq!(a.get("masks.deployed"), Some(0.0));
    a.set("eng1.on", 0.0);
    run(&mut a, 600.0);
    assert_eq!(a.get("masks.deployed"), Some(1.0), "with no inflow the cabin climbs past 14000 ft");
}

#[test]
fn ice_follows_its_heat_balance() {
    let mut a = aircraft();
    a.set("air.icing", 1.0);
    a.set("air.airspeed", 51.44);
    a.set("air.temperature", 263.15);
    run(&mut a, 20.0);
    let glaze = a.get("wing.freezing_fraction").unwrap();
    let t1 = a.get("wing.thickness").unwrap();
    assert!(t1 > 0.0 && glaze > 0.0 && glaze < 1.0, "at -10 C and heavy icing part of the water runs back: n = {glaze:.2}");
    assert!((a.get("wing.surface_temperature").unwrap() - 273.15).abs() < 0.01, "a glazing surface sits at 0 C");
    a.set("air.temperature", 238.15);
    a.set("air.icing", 0.2);
    run(&mut a, 20.0);
    assert!(a.get("wing.freezing_fraction").unwrap() > glaze, "colder and drier freezes more of what it catches");
    a.set("air.temperature", 258.15);
    a.set("air.airspeed", 240.0);
    let before = a.get("wing.thickness").unwrap();
    run(&mut a, 20.0);
    assert!(a.get("wing.surface_temperature").unwrap() > 273.15, "kinetic heating at 470 kt keeps the surface above freezing");
    assert!(a.get("wing.thickness").unwrap() < before, "and the ice melts back");
    a.set("air.airspeed", 51.44);
    a.set("eng1.on", 1.0);
    run(&mut a, 60.0);
    assert_eq!(a.get("wing.thickness"), Some(0.0), "the heated wing sheds its ice");
    assert_eq!(a.get("wing.heated"), Some(1.0));
}
