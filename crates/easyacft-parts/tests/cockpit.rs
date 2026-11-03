use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const COCKPIT: &str = r#"
[[cockpit.switch]]
id = "ann_test"

[[cockpit.guarded_switch]]
id = "fire_handle"

[[cockpit.pushbutton]]
id = "gen1_pb"
initial = true
fault_when = "!gen_ok.on"
test = "ann_test"

[[logic.flag]]
id = "gen_ok"
when = "true"

[[cockpit.selector]]
id = "eng_mode"
positions = 3
initial = 1

[[cockpit.annunciator]]
id = "master_caution"
when = "!gen1_pb.pressed"
test = "ann_test"

[[weight.station]]
id = "empty"
mass = "1000 kg"
arm = "2.0 m"

[[weight.station]]
id = "pilot"
mass = "80 kg"
arm = "1.5 m"

[[weight.station]]
id = "bags"
mass = "20 kg"
arm = "3.5 m"

[[weight.balance]]
id = "wb"
stations = ["empty", "pilot", "bags"]
mac_leading_edge = "1.8 m"
mac = "1.5 m"
"#;

fn aircraft() -> Aircraft {
    let (m, mut d) = load_str("cockpit", COCKPIT);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

#[test]
fn guards_lights_selectors_and_balance() {
    let mut a = aircraft();
    a.set("fire_handle.on", 1.0);
    a.step(0.05);
    assert_eq!(a.get("fire_handle.on"), Some(0.0), "the guard is closed");
    a.set("fire_handle.guard_open", 1.0);
    a.set("fire_handle.on", 1.0);
    a.step(0.05);
    assert_eq!(a.get("fire_handle.on"), Some(1.0));
    assert_eq!(a.get("gen1_pb.on_light"), Some(1.0));
    assert_eq!(a.get("gen1_pb.fault_light"), Some(0.0));
    assert_eq!(a.get("master_caution.lit"), Some(0.0));
    a.set("gen1_pb.pressed", 0.0);
    a.step(0.05);
    assert_eq!(a.get("master_caution.lit"), Some(1.0));
    a.set("ann_test.on", 1.0);
    a.step(0.05);
    assert_eq!(a.get("gen1_pb.fault_light"), Some(1.0), "the test lights every light");
    a.set("eng_mode.position", 7.0);
    a.step(0.05);
    assert_eq!(a.get("eng_mode.position"), Some(2.0));
    let cg = (1000.0 * 2.0 + 80.0 * 1.5 + 20.0 * 3.5) / 1100.0;
    assert!((a.get("wb.mass").unwrap() - 1100.0).abs() < 1e-9);
    assert!((a.get("wb.cg").unwrap() - cg).abs() < 1e-9);
    assert!((a.get("wb.cg_mac").unwrap() - (cg - 1.8) / 1.5 * 100.0).abs() < 1e-9);
    a.set("pilot.mass", 0.0);
    a.step(0.05);
    assert!((a.get("wb.mass").unwrap() - 1020.0).abs() < 1e-9);
}
