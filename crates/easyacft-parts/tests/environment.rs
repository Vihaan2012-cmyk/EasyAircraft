use easyacft_lang::load_str;
use easyacft_parts::Aircraft;

fn aircraft(src: &str) -> Aircraft {
    let (m, mut d) = load_str("environment", src);
    match Aircraft::from_model(m, &mut d) {
        Ok((a, _)) => a,
        Err(r) => panic!("{}", r.render()),
    }
}

fn get(a: &Aircraft, k: &str) -> f64 {
    a.get(k).unwrap()
}

#[test]
fn a_standard_atmosphere_gives_cruise_air_data() {
    let mut a = aircraft("[[environment.air]]\nid = \"air\"\nstandard = true\n");
    a.set("air.altitude", 10_668.0);
    a.set("air.airspeed", 230.0);
    a.step(0.02);
    let (p, t, m) = (get(&a, "air.pressure"), get(&a, "air.temperature"), get(&a, "air.mach"));
    assert!((p - 23_842.0).abs() < 25.0 && (t - 218.8).abs() < 0.2, "FL350 {p:.0} Pa {t:.1} K");
    assert!((m - 0.776).abs() < 0.005, "mach {m:.3}");
    let (eas, cas) = (get(&a, "air.eas"), get(&a, "air.cas"));
    assert!((eas - 128.0).abs() < 1.0, "eas {eas:.1}");
    assert!((cas - 135.2).abs() < 0.5 && cas > eas, "cas {cas:.1} above eas {eas:.1} by the compressibility correction");
    assert!((get(&a, "air.pressure_altitude") - 10_668.0).abs() < 2.0);
    let tat = get(&a, "air.total_temperature");
    assert!((tat - t * (1.0 + 0.2 * m * m)).abs() < 0.01 && tat > t + 20.0, "ram rise {t:.1} -> {tat:.1}");
}

#[test]
fn a_hot_day_raises_density_altitude_and_groundspeed_stands_in_for_airspeed() {
    let mut a = aircraft("[[environment.air]]\nid = \"air\"\nstandard = true\nisa_deviation = \"20 K\"\nwind_speed = \"10 m/s\"\n");
    a.set("air.groundspeed", 50.0);
    a.step(0.02);
    assert!((get(&a, "air.temperature") - 308.15).abs() < 0.01);
    assert!((get(&a, "air.airspeed") - 60.0).abs() < 1e-9, "groundspeed plus headwind");
    let da = get(&a, "air.density_altitude");
    assert!((600.0..850.0).contains(&da), "density altitude {da:.0} m");
    assert!(get(&a, "air.pressure_altitude").abs() < 1.0);
    a.set("air.groundspeed", 70.0);
    a.step(0.02);
    assert!((get(&a, "air.airspeed") - 80.0).abs() < 1e-9, "follows groundspeed");
}

#[test]
fn bound_air_keeps_its_values_and_derives_the_rest() {
    let mut a = aircraft("[[environment.air]]\nid = \"air\"\n");
    a.set("air.pressure", 101_325.0);
    a.set("air.temperature", 288.15);
    a.set("air.airspeed", 100.0);
    a.set("air.mach", 0.123);
    a.step(0.02);
    assert_eq!(a.get("air.mach"), Some(0.123), "bound mach is left alone");
    let cas = get(&a, "air.cas");
    assert!((cas - 100.0).abs() < 0.2, "sea level CAS equals TAS: {cas:.2}");
    assert!((get(&a, "air.density") - 1.225).abs() < 0.001);
    assert!((get(&a, "air.dynamic_pressure") - 6125.0).abs() < 5.0);
    assert!(get(&a, "air.total_pressure") > 101_325.0 + 6125.0, "compressible total pressure");
}
