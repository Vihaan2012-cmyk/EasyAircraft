use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

const APU: &str = r#"
[[environment.air]]
id = "air"

[[cockpit.switch]]
id = "master"

[[cockpit.switch]]
id = "start"

[[cockpit.switch]]
id = "dead_battery"

[[cockpit.switch]]
id = "oil_leak"

[[apu.unit]]
id = "apu"
air = "air"
master = "master.on"
start = "start.on"
start_time = "40 s"
door_time = "15 s"
starter_when = "!dead_battery.on"
start_ceiling = "25000 ft"
oil_failed_when = "oil_leak.on"
"#;

fn aircraft() -> Aircraft {
    let (model, mut d) = load_str("apu", APU);
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

fn start(a: &mut Aircraft) {
    a.set("master.on", 1.0);
    a.set("start.on", 1.0);
    run(a, 0.5);
    a.set("start.on", 0.0);
}

#[test]
fn the_door_opens_then_it_lights_off_peaks_and_governs() {
    let mut a = aircraft();
    start(&mut a);
    run(&mut a, 10.0);
    assert_eq!(a.get("apu.n"), Some(0.0), "the inlet door opens first");
    assert!(a.get("apu.door").unwrap() > 0.5);
    run(&mut a, 10.0);
    assert_eq!(a.get("apu.starter"), Some(1.0));
    let mut peak: f64 = 0.0;
    for _ in 0..400 {
        run(&mut a, 0.1);
        peak = peak.max(a.get("apu.egt").unwrap());
    }
    assert_eq!(a.get("apu.available"), Some(1.0));
    assert_eq!(a.get("apu.starter"), Some(0.0), "cut out past 55 %");
    let governed = a.get("apu.egt").unwrap();
    assert!(peak > governed + 50.0, "the start EGT peaks above the governed value: {peak:.0} vs {governed:.0} K");
    let oil = a.get("apu.oil_pressure").unwrap() / 6894.757;
    assert!((50.0..65.0).contains(&oil), "oil {oil:.0} psi at 100 %");
}

#[test]
fn start_faults_and_protective_shutdowns_say_why() {
    let mut a = aircraft();
    a.set("dead_battery.on", 1.0);
    start(&mut a);
    run(&mut a, 90.0);
    assert_eq!((a.get("apu.shutdown"), a.get("apu.reason"), a.get("apu.start_fault")), (Some(1.0), Some(4.0), Some(1.0)), "hung with no starter power");
    let mut b = aircraft();
    b.set("air.pressure", 30_000.0);
    start(&mut b);
    run(&mut b, 1.0);
    assert_eq!(b.get("apu.reason"), Some(7.0), "above its start ceiling");
    let mut c = aircraft();
    start(&mut c);
    run(&mut c, 70.0);
    assert_eq!(c.get("apu.available"), Some(1.0));
    c.set("oil_leak.on", 1.0);
    run(&mut c, 1.0);
    assert_eq!((c.get("apu.shutdown"), c.get("apu.reason")), (Some(1.0), Some(5.0)), "low oil pressure");
}

#[test]
fn thin_air_costs_bleed_and_a_hot_day_raises_egt() {
    let mut a = aircraft();
    start(&mut a);
    run(&mut a, 70.0);
    let cool = a.get("apu.egt").unwrap();
    a.set("air.temperature", 318.15);
    a.set("air.pressure", 60_000.0);
    run(&mut a, 30.0);
    assert!(a.get("apu.egt").unwrap() > cool + 30.0);
    let bleed = a.get("apu.bleed_available").unwrap();
    assert!((bleed - 60_000.0 / 101_325.0).abs() < 0.01, "bleed falls with density: {bleed:.2}");
}
