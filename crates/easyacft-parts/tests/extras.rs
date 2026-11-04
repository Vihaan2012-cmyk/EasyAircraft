use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const SRC: &str = r#"
[[cockpit.switch]]
id = "beacon_sw"

[[cockpit.switch]]
id = "overheat"

[[cockpit.pushbutton]]
id = "agent_pb"

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.battery]]
id = "bat"
bus = "dc"
voltage = "24 V"
capacity = "40 Ah"

[[light.unit]]
id = "beacon"
bus = "dc"
on_when = "beacon_sw.on"
pattern = "beacon"

[[failure.trigger]]
id = "bat_fail"
part = "bat"
when = "delay(overheat.on, 2 s)"

[[fire.zone]]
id = "eng_fire"
fire_when = "overheat.on"

[[fire.bottle]]
id = "bottle"
zones = ["eng_fire"]
discharge_when = "agent_pb.pressed"

[[sound.event]]
id = "click"
trigger = "beacon_sw.on"

[[logic.value]]
id = "volts"
value = "dc.voltage / 1 V"
"#;

fn aircraft() -> Aircraft {
    let (m, mut d) = load_str("extras", SRC);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn run(a: &mut Aircraft, s: f64) -> Vec<f64> {
    let mut lit = Vec::new();
    for _ in 0..(s * 50.0) as usize {
        a.step(0.02);
        lit.push(a.get("beacon.lit").unwrap());
    }
    lit
}

#[test]
fn lights_flash_sounds_pulse_values_follow() {
    let mut a = aircraft();
    run(&mut a, 0.5);
    assert!(a.get("volts.value").unwrap() > 20.0);
    a.set("beacon_sw.on", 1.0);
    a.step(0.02);
    assert_eq!(a.get("click.play"), Some(1.0));
    let lit = run(&mut a, 2.4);
    assert_eq!(a.get("click.play"), Some(0.0));
    assert_eq!(a.get("click.count"), Some(1.0));
    let on: f64 = lit.iter().sum::<f64>() / lit.len() as f64;
    assert!(on > 0.05 && on < 0.25, "a beacon is lit part of the time: {on:.2}");
}

#[test]
fn failures_trigger_and_fires_go_out_with_agent() {
    let mut a = aircraft();
    a.set("overheat.on", 1.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("eng_fire.detected"), Some(0.0), "the loops take their time to heat");
    assert_eq!(a.get("bat.failed"), Some(0.0));
    run(&mut a, 1.5);
    assert_eq!(a.get("bat.failed"), Some(1.0), "the trigger failed the battery after its delay");
    run(&mut a, 0.5);
    assert!(a.get("dc.voltage").unwrap() < 1.0);
    run(&mut a, 2.0);
    assert_eq!(a.get("eng_fire.detected"), Some(1.0), "both loops in alarm within 5 s");
    assert!(a.get("eng_fire.temperature").unwrap() > 673.15);
    a.set("agent_pb.pressed", 1.0);
    run(&mut a, 4.0);
    assert_eq!(a.get("eng_fire.burning"), Some(0.0));
    assert_eq!(a.get("bottle.discharged"), Some(1.0));
    a.set("overheat.on", 0.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("bat.failed"), Some(1.0), "latched");
}
