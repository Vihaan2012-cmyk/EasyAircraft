use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const AV: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "fan"

[[cockpit.switch]]
id = "heat"

[[cockpit.switch]]
id = "pos"

[[computer.unit]]
id = "adiru"
channels = 2
transfer_time = "0.5 s"
load = "120 W"
boot_time = "2 s"
cooled_when = "fan.on"
overheat_time = "60 s"

[[data.link]]
id = "alt_word"
from = "adiru"
value = "air.altitude / 0.3048 m"
label = 203
range = 131072
bits = 17
valid_when = "pos.on"

[[sensor.pitot]]
id = "pitot"
air = "air"

[[sensor.static_port]]
id = "static"
air = "air"
error = 0.01
sideslip_error = 0.5

[[sensor.tat]]
id = "tat"
air = "air"
heat_when = "heat.on"

[[sensor.aoa]]
id = "vane"
air = "air"
freeze_time = "60 s"

[[computer.air_data]]
id = "adr"
pitot = "pitot"
static = "static"
air = "air"
tat_probe = "tat"
vmo = "340 kt"
mmo = 0.89

[[computer.inertial]]
id = "ir"
air = "air"
align_time = "600 s"
position_entered = "pos.on"

[[wear.counter]]
id = "tyre_wear"
cycle_when = "air.on_ground"
per_cycle = 1
life = 300
shape = 3
limit = 250
"#;

fn aircraft() -> Aircraft {
    let (model, mut d) = load_str("av", AV);
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

#[test]
fn an_arinc_word_carries_label_value_status_and_parity() {
    let mut a = aircraft();
    a.set("fan.on", 1.0);
    a.set("air.altitude", 10_000.0 * 0.3048);
    run(&mut a, 3.0);
    assert_eq!(a.get("alt_word.ssm"), Some(1.0), "no position entered: no computed data");
    assert_eq!(a.get("alt_word.valid"), Some(0.0));
    a.set("pos.on", 1.0);
    run(&mut a, 0.5);
    assert_eq!(a.get("alt_word.ssm"), Some(3.0));
    let w = a.get("alt_word.word").unwrap() as u32;
    assert_eq!(w.count_ones() % 2, 1, "odd parity");
    assert_eq!((w & 0xFF).reverse_bits() >> 24, 0o203, "label 203 sent bit-reversed");
    assert_eq!((w >> 29) & 3, 3, "normal operation");
    let counts = ((w >> 10) & 0x7FFFF) as f64 / 4.0;
    assert!((counts - 10_000.0).abs() < 1.0, "17 bits over 131072 ft is 1 ft: {counts}");
    a.set("air.altitude", 200_000.0 * 0.3048);
    run(&mut a, 0.5);
    assert!(a.get("alt_word.value").unwrap() < 131_072.0, "beyond its range the word clips");
}

#[test]
fn a_computer_overheats_without_cooling_and_hands_over_channels() {
    let mut a = aircraft();
    a.set("fan.on", 1.0);
    run(&mut a, 3.0);
    assert_eq!(a.get("adiru.running"), Some(1.0));
    assert_eq!(a.get("adiru.load"), Some(120.0));
    a.set("adiru.channel_failed", 1.0);
    run(&mut a, 0.1);
    assert_eq!(a.get("adiru.transferring"), Some(1.0));
    assert_eq!(a.get("adiru.running"), Some(0.0), "output lost while channel 2 takes over");
    run(&mut a, 1.0);
    assert_eq!((a.get("adiru.running"), a.get("adiru.channel"), a.get("adiru.degraded")), (Some(1.0), Some(2.0), Some(1.0)));
    a.set("fan.on", 0.0);
    run(&mut a, 70.0);
    assert_eq!(a.get("adiru.overheat"), Some(1.0));
    assert_eq!(a.get("adiru.running"), Some(0.0), "it shuts itself down");
}

#[test]
fn air_data_gives_cas_tas_and_overspeed_and_the_tat_probe_reads_high_on_the_ground() {
    let mut a = aircraft();
    a.set("fan.on", 1.0);
    a.set("heat.on", 1.0);
    run(&mut a, 20.0);
    assert!(a.get("tat.temperature").unwrap() > 288.15 + 15.0, "heated and still, the probe reads its heater");
    a.set("air.pressure", 23_842.0);
    a.set("air.temperature", 218.8);
    a.set("air.airspeed", 252.0);
    a.set("air.mach", 0.85);
    run(&mut a, 20.0);
    let tas = a.get("adr.tas").unwrap();
    assert!((tas - 252.0).abs() < 5.0, "TAS from Mach and SAT: {tas:.1} m/s");
    let cas = a.get("adr.cas").unwrap() / 0.514444;
    assert!((250.0..300.0).contains(&cas), "M0.85 at FL350 is about 275 kt CAS: {cas:.0}");
    let sat = a.get("adr.sat").unwrap();
    assert!((sat - 218.8).abs() < 3.0, "SAT back out of the TAT probe: {sat:.1}");
    assert_eq!(a.get("adr.overspeed"), Some(0.0));
    a.set("air.mach", 0.92);
    a.set("air.airspeed", 272.0);
    run(&mut a, 5.0);
    assert_eq!(a.get("adr.overspeed"), Some(1.0), "past MMO 0.89");
}

#[test]
fn an_unheated_vane_freezes_and_the_irs_takes_longer_far_north() {
    let mut a = aircraft();
    a.set("air.aoa", 3.0);
    run(&mut a, 2.0);
    assert!((a.get("vane.angle").unwrap() - 3.0).abs() < 0.1);
    a.set("air.icing", 1.0);
    a.set("air.temperature", 258.15);
    run(&mut a, 65.0);
    a.set("air.aoa", 12.0);
    run(&mut a, 2.0);
    assert_eq!(a.get("vane.frozen"), Some(1.0));
    assert!((a.get("vane.angle").unwrap() - 3.0).abs() < 0.2, "frozen in place");
    let mut b = aircraft();
    b.set("fan.on", 1.0);
    b.set("air.latitude", 75.0);
    b.set("pos.on", 1.0);
    run(&mut b, 700.0);
    assert_eq!(b.get("ir.aligned"), Some(0.0), "at 75 N it needs more than 10 minutes");
    run(&mut b, 350.0);
    assert_eq!(b.get("ir.aligned"), Some(1.0));
    let mut c = aircraft();
    c.set("pos.on", 1.0);
    run(&mut c, 300.0);
    c.set("air.groundspeed", 3.0);
    run(&mut c, 1.0);
    c.set("air.groundspeed", 0.0);
    run(&mut c, 400.0);
    assert_eq!(c.get("ir.aligned"), Some(0.0), "moving restarted the alignment");
}

#[test]
fn wear_counts_cycles_toward_its_life() {
    let mut a = aircraft();
    for _ in 0..200 {
        a.set("air.on_ground", 0.0);
        run(&mut a, 0.1);
        a.set("air.on_ground", 1.0);
        run(&mut a, 0.1);
    }
    assert_eq!(a.get("tyre_wear.cycles"), Some(200.0));
    let pf = a.get("tyre_wear.failure_probability").unwrap();
    let want = 1.0 - (-(200.0f64 / 300.0).powi(3)).exp();
    assert!((pf - want).abs() < 0.01, "Weibull at 200 of 300 cycles: {pf:.3}");
    assert_eq!(a.get("tyre_wear.worn"), Some(0.0));
}
