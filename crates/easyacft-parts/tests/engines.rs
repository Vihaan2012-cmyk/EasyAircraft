use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const JET: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "master"

[[cockpit.switch]]
id = "start_sel"

[[cockpit.selector]]
id = "lever"
positions = 101

[[fuel.tank]]
id = "tank"
capacity = "5000 kg"
quantity = "4000 kg"

[[fuel.line]]
id = "feed_line"

[[fuel.pump]]
id = "boost"
tank = "tank"
outlet = "feed_line"
flow = "3 kg/s"

[[fuel.feed]]
id = "feed"
line = "feed_line"
demand = "fadec.fuel_command"

[[engine.gas_turbine]]
id = "eng"
air = "air"
fuel_flow = "feed.flow"
design_thrust = "120 kN"
airflow = "400 kg/s"
bypass_ratio = 5.5
fan_pr = 1.6
lpc_pr = 1.4
hpc_pr = 14
lp_rpm = "5000 rpm"
hp_rpm = "14000 rpm"
lp_inertia = 60
hp_inertia = 8
starter_torque = "if(fadec.starter, 1200 N*m, 0 N*m)"
reverser = "fadec.reverser"

[[fadec.controller]]
id = "fadec"
engine = "eng"
lever = "lever.position / 100"
master = "master.on"
start = "start_sel.on"
idle = "25"
max = "100"
fuel_on_at = 12
starter_cutout = 45

[[apu.unit]]
id = "apu"
master = "master.on"
start = "start_sel.on"

[[output.force]]
id = "eng_force"
force = "eng.thrust"
x = "5 m"
y = "-2 m"
z = "0 m"
"#;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("engines", src);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) {
    for _ in 0..(s * 50.0) as usize {
        a.step(0.02);
    }
}

#[test]
fn the_fadec_starts_the_engine_and_follows_the_lever() {
    let mut a = aircraft(JET);
    a.set("master.on", 1.0);
    a.set("start_sel.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("fadec.state"), Some(1.0), "cranking");
    assert_eq!(a.get("fadec.starter"), Some(1.0));
    run(&mut a, 90.0);
    let n2 = a.get("eng.n2").unwrap();
    assert_eq!(a.get("fadec.state"), Some(3.0), "running, n2 {n2:.1}");
    assert_eq!(a.get("fadec.starter"), Some(0.0));
    a.set("start_sel.on", 0.0);
    run(&mut a, 30.0);
    let idle = a.get("eng.n1").unwrap();
    assert!((idle - 25.0).abs() < 3.0, "idle n1 {idle:.1}");
    a.set("lever.position", 100.0);
    run(&mut a, 40.0);
    let n1 = a.get("eng.n1").unwrap();
    let thrust = a.get("eng.thrust").unwrap();
    assert!((n1 - 100.0).abs() < 3.0, "full lever n1 {n1:.1}");
    assert!(thrust > 100_000.0, "thrust {thrust:.0}");
    assert!((a.get("eng_force.force").unwrap() - thrust).abs() < 1.0);
    assert!(a.get("tank.quantity").unwrap() < 4000.0, "fuel burnt");
    a.set("master.on", 0.0);
    run(&mut a, 60.0);
    assert!(a.get("eng.n2").unwrap() < 20.0, "spun down");
}

#[test]
fn component_damage_changes_the_gas_path() {
    let mut a = aircraft(JET);
    a.set("master.on", 1.0);
    a.set("start_sel.on", 1.0);
    run(&mut a, 90.0);
    a.set("start_sel.on", 0.0);
    a.set("lever.position", 80.0);
    run(&mut a, 40.0);
    let (egt, n2, ff, thrust) = (a.get("eng.egt").unwrap(), a.get("eng.n2").unwrap(), a.get("eng.fuel_flow").unwrap(), a.get("eng.thrust").unwrap());
    a.set("eng.hpt_damage", 0.6);
    run(&mut a, 30.0);
    let egt_hot = a.get("eng.egt").unwrap();
    assert!(egt_hot > egt + 20.0 || a.get("eng.fuel_flow").unwrap() > ff * 1.05, "turbine damage runs hotter or burns more: egt {egt:.0} -> {egt_hot:.0}");
    a.set("eng.hpt_damage", 0.0);
    a.set("eng.surge", 1.0);
    run(&mut a, 10.0);
    let (n2s, egts, ths) = (a.get("eng.n2").unwrap(), a.get("eng.egt").unwrap(), a.get("eng.thrust").unwrap());
    let why = format!("n2 {n2:.1} -> {n2s:.1}, egt {egt:.0} -> {egts:.0}, thrust {thrust:.0} -> {ths:.0}");
    assert!(ths < thrust * 0.7 && egts > egt + 50.0 && n2s < n2 + 1.0, "surge: thrust lost, EGT up, core hangs: {why}");
    a.set("eng.surge", 0.0);
    a.set("eng.seized_hp", 1.0);
    run(&mut a, 5.0);
    assert_eq!(a.get("eng.n2"), Some(0.0), "seized spool stops");
    assert_eq!(a.get("eng.running"), Some(0.0));
}

#[test]
fn the_apu_starts_and_becomes_available() {
    let mut a = aircraft(JET);
    a.set("master.on", 1.0);
    a.set("start_sel.on", 1.0);
    run(&mut a, 20.0);
    assert_eq!(a.get("apu.starting"), Some(1.0));
    run(&mut a, 40.0);
    assert_eq!(a.get("apu.available"), Some(1.0));
    assert!(a.get("apu.egt").unwrap() > 600.0);
}

const PISTON: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.selector]]
id = "throttle"
positions = 101

[[cockpit.switch]]
id = "starter"

[[engine.piston]]
id = "eng"
air = "air"
rated_power = "180 hp"
rpm = "prop.rpm"
throttle = "throttle.position / 100"

[[engine.propeller]]
id = "prop"
air = "air"
power = "eng.power"
diameter = "1.9 m"
rated_rpm = "2700 rpm"
rated_power = "180 hp"
starter_torque = "if(starter.on, 150 N*m, 0 N*m)"
friction = "8 kW"
"#;

#[test]
fn a_piston_engine_starts_and_its_propeller_makes_thrust() {
    let mut a = aircraft(PISTON);
    a.set("throttle.position", 20.0);
    a.set("starter.on", 1.0);
    run(&mut a, 3.0);
    a.set("starter.on", 0.0);
    run(&mut a, 10.0);
    assert_eq!(a.get("eng.running"), Some(1.0), "rpm {}", a.get("prop.rpm").unwrap() * 60.0);
    a.set("throttle.position", 100.0);
    run(&mut a, 15.0);
    let rpm = a.get("prop.rpm").unwrap() * 60.0;
    assert!((2300.0..2900.0).contains(&rpm), "full throttle rpm {rpm:.0}");
    let t = a.get("prop.thrust").unwrap();
    assert!((2500.0..4500.0).contains(&t), "static thrust {t:.0} N");
    assert!(a.get("eng.fuel_flow").unwrap() > 0.005);
}

#[test]
fn a_flameout_brings_the_igniters_on() {
    let mut a = aircraft(JET);
    a.set("master.on", 1.0);
    a.set("start_sel.on", 1.0);
    run(&mut a, 90.0);
    a.set("start_sel.on", 0.0);
    run(&mut a, 10.0);
    assert_eq!((a.get("fadec.flameout"), a.get("fadec.ignition")), (Some(0.0), Some(0.0)));
    a.set("eng.failed", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("fadec.flameout"), Some(1.0));
    assert_eq!(a.get("fadec.ignition"), Some(1.0), "auto-relight");
}
