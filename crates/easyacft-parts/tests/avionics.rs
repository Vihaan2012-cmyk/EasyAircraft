use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const SRC: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "bat_sw"
initial = true

[[cockpit.switch]]
id = "pitot_heat"

[[cockpit.switch]]
id = "parked"
initial = true

[[electrical.bus]]
id = "dc"
nominal = "28 V"

[[electrical.battery]]
id = "bat"
bus = "hot"
voltage = "24 V"
capacity = "40 Ah"

[[electrical.bus]]
id = "hot"
nominal = "28 V"

[[electrical.contactor]]
id = "blc"
between = ["hot", "dc"]
close_when = "bat_sw.on"

[[computer.unit]]
id = "adc1"
power = ["dc"]
boot_time = "3 s"
channels = 2

[[data.link]]
id = "alt_word"
from = "adc1"
value = "adc.altitude / 1 m"
rate = "10 Hz"
latency = "0.1 s"

[[sensor.pitot]]
id = "pitot"
air = "air"
heat_when = "pitot_heat.on"

[[sensor.static_port]]
id = "static"
air = "air"

[[computer.air_data]]
id = "adc"
pitot = "pitot"
static = "static"
air = "air"
computer = "adc1"

[[computer.inertial]]
id = "irs"
air = "air"
computer = "adc1"
align_time = "10 s"

[[wear.counter]]
id = "brakes"
rate = "if(parked.on, 0.01, 0)"
limit = 0.5
service_when = "false"
"#;

fn aircraft() -> Aircraft {
    let (m, mut d) = load_str("avionics", SRC);
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
fn a_computer_boots_feeds_its_links_and_loses_them_without_power() {
    let mut a = aircraft();
    a.set("air.airspeed", 100.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("adc1.booting"), Some(1.0));
    assert_eq!(a.get("alt_word.valid"), Some(0.0));
    run(&mut a, 3.0);
    assert_eq!(a.get("adc1.running"), Some(1.0));
    let ias = a.get("adc.ias").unwrap();
    assert!((ias - 100.0).abs() < 1.0, "ias {ias:.1} m/s at sea level");
    assert_eq!(a.get("alt_word.valid"), Some(1.0));
    a.set("adc1.channel_failed", 1.0);
    run(&mut a, 0.5);
    assert_eq!(a.get("adc1.channel"), Some(2.0), "hands over to channel B");
    assert_eq!(a.get("adc1.running"), Some(1.0));
    a.set("bat_sw.on", 0.0);
    run(&mut a, 1.0);
    assert_eq!(a.get("adc1.running"), Some(0.0));
    assert_eq!(a.get("alt_word.status"), Some(2.0), "failure warning once the sender is down");
    assert_eq!(a.get("adc.valid"), Some(0.0));
}

#[test]
fn an_iced_pitot_freezes_the_airspeed_and_heat_clears_it() {
    let mut a = aircraft();
    a.set("air.airspeed", 100.0);
    a.set("air.temperature", 263.15);
    run(&mut a, 4.0);
    let before = a.get("adc.ias").unwrap();
    a.set("air.icing", 1.0);
    run(&mut a, 10.0);
    assert_eq!(a.get("pitot.iced"), Some(0.0), "the probe has to cool below freezing and ice over first");
    run(&mut a, 60.0);
    assert_eq!(a.get("pitot.iced"), Some(1.0));
    a.set("air.airspeed", 150.0);
    run(&mut a, 1.0);
    assert!((a.get("adc.ias").unwrap() - before).abs() < 0.5, "blocked: ias stays at {before:.1}");
    a.set("pitot_heat.on", 1.0);
    run(&mut a, 30.0);
    assert!(a.get("pitot.temperature").unwrap() > 273.15, "the heater takes the probe above freezing");
    assert!(a.get("adc.ias").unwrap() > before + 40.0);
}

#[test]
fn the_irs_aligns_when_still_and_failures_reach_the_log() {
    let mut a = aircraft();
    run(&mut a, 5.0);
    assert_eq!(a.get("irs.aligning"), Some(1.0));
    run(&mut a, 10.0);
    assert_eq!(a.get("irs.aligned"), Some(1.0));
    a.set("air.heading", 1.2);
    run(&mut a, 0.1);
    assert_eq!(a.get("irs.heading"), Some(1.2));
    a.set("bat.failed", 1.0);
    run(&mut a, 0.1);
    assert!(a.log.iter().any(|e| e.what == "bat.failed" && e.on_ground), "{:?}", a.log);
    assert!((a.get("brakes.value").unwrap() - 0.152).abs() < 0.01);
    let saved = a.persistent();
    let mut b = aircraft();
    assert_eq!(b.restore_persistent(&saved), 1);
    assert!((b.get("brakes.value").unwrap() - a.get("brakes.value").unwrap()).abs() < 1e-9);
}
