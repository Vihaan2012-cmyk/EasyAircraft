//! The plugin A380's hydraulic users as real parts: every flight control surface on its actuators, the stabiliser,
//! the high-lift drive lines, five gear legs with doors and bogies, sixteen braked wheels and the steering, each
//! failure from the plugin's catalogue wired to the field it breaks.

use std::fmt::Write;

/// One surface: catalogue id, our id, where its two actuators draw from (g, y or e for an EHA), travel stops,
/// command source and its deflection convention.
struct Surface {
    cat: &'static str,
    id: &'static str,
    supplies: &'static [char],
    min: f64,
    max: f64,
    command: &'static str,
    deflects: &'static str,
    area: f64,
}

/// Actuator supplies follow the plugin's deep flight controls (allocation.rs, live.rs spoiler_is_green).
const SURFACES: &[Surface] = &[
    Surface { cat: "ail_l1", id: "ail_l_out", supplies: &['g', 'y'], min: -30.0, max: 30.0, command: "-roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "ail_l2", id: "ail_l_mid", supplies: &['y', 'e'], min: -30.0, max: 30.0, command: "-roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "ail_l3", id: "ail_l_in", supplies: &['g', 'e'], min: -30.0, max: 30.0, command: "-roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "ail_r1", id: "ail_r_out", supplies: &['g', 'y'], min: -30.0, max: 30.0, command: "roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "ail_r2", id: "ail_r_mid", supplies: &['y', 'e'], min: -30.0, max: 30.0, command: "roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "ail_r3", id: "ail_r_in", supplies: &['g', 'e'], min: -30.0, max: 30.0, command: "roll_cmd.value", deflects: "down", area: 5.0 },
    Surface { cat: "elev_l_inbd", id: "elev_l_in", supplies: &['g', 'e'], min: -30.0, max: 20.0, command: "-pitch_cmd.value", deflects: "down", area: 9.0 },
    Surface { cat: "elev_l_outbd", id: "elev_l_out", supplies: &['g', 'e'], min: -30.0, max: 20.0, command: "-pitch_cmd.value", deflects: "down", area: 9.0 },
    Surface { cat: "elev_r_inbd", id: "elev_r_in", supplies: &['g', 'e'], min: -30.0, max: 20.0, command: "-pitch_cmd.value", deflects: "down", area: 9.0 },
    Surface { cat: "elev_r_outbd", id: "elev_r_out", supplies: &['g', 'e'], min: -30.0, max: 20.0, command: "-pitch_cmd.value", deflects: "down", area: 9.0 },
    Surface { cat: "rud_upper", id: "rud_upper", supplies: &['y', 'g'], min: -30.0, max: 30.0, command: "yaw_cmd.value", deflects: "right", area: 15.0 },
    Surface { cat: "rud_lower", id: "rud_lower", supplies: &['g', 'y'], min: -30.0, max: 30.0, command: "yaw_cmd.value", deflects: "right", area: 15.0 },
];

/// The plugin's failure words for a surface component, by the field they break.
const SURFACE_FAILS: &[(&str, &[&str])] = &[
    ("jam", &["actuator jam"]),
    ("runaway", &["hardover"]),
    ("supply", &["supply loss"]),
    ("sensor", &["transducer fault", "transducer open circuit", "transducer intermittent"]),
    ("drift", &["transducer drift"]),
    ("disconnect", &["disconnect"]),
    ("valve_leak", &["internal leakage"]),
    ("seal", &["seal wear"]),
    ("damper", &["flutter damper"]),
];

fn circuit_of(s: char) -> &'static str {
    if s == 'y' { "yellow" } else { "green" }
}

fn when(k: &str, e: &str) -> String {
    if e == "false" { String::new() } else { format!("{k} = \"{e}\"\n") }
}

fn or(a: &str, b: &str) -> String {
    match (a, b) {
        ("false", _) => b.to_owned(),
        (_, "false") => a.to_owned(),
        _ => format!("{a} || {b}"),
    }
}

fn actuator(h: &mut String, id: &str, supply: char, force: &str, rate: &str, extra: &str) {
    let body = match supply {
        'e' => "control = \"eha\"\npowered_when = \"ac_ess.powered\"\n".to_owned(),
        s => format!("circuit = \"{}\"\nnull_leak = \"0.25 L/min\"\n", circuit_of(s)),
    };
    writeln!(h, "[[actuator.linear]]\nid = \"{id}\"\n{body}force = \"{force}\"\nrate = \"{rate}\"\n{extra}").unwrap();
}

/// Everything the two circuits drive, as TOML; `f(component, words)` is the catalogue failure expression.
pub fn users(f: &dyn Fn(&str, &[&str]) -> String) -> String {
    let mut h = String::new();
    h += "# What the crew and the flight control computers command: deflections in degrees, normalised levers.\n";
    for id in ["roll_cmd", "pitch_cmd", "yaw_cmd", "speedbrake_cmd", "ths_cmd", "flap_lever", "slat_lever", "brake_left", "brake_right", "steer_deg"] {
        writeln!(h, "[[input.number]]\nid = \"{id}\"\n").unwrap();
    }
    h += "[[cockpit.switch]]\nid = \"parking_brake\"\ninitial = true\n\n[[cockpit.switch]]\nid = \"gear_gravity\"\n\n[[cockpit.switch]]\nid = \"towing\"\n\n";

    h += "# Ailerons, elevators and rudders: two actuators each, the first active and the second damping until the computers hand over; EHAs on AC ESS.\n";
    for s in SURFACES {
        let comp = format!("27_fctl.{}", s.cat);
        let fx = |k: &str| SURFACE_FAILS.iter().find(|(n, _)| *n == k).map(|(_, w)| f(&comp, w)).unwrap_or_else(|| "false".into());
        let (jam, runaway, supply, sensor, drift, disconnect, valve_leak, seal) = (fx("jam"), fx("runaway"), fx("supply"), fx("sensor"), fx("drift"), fx("disconnect"), fx("valve_leak"), fx("seal"));
        let mut ids = Vec::new();
        for (k, &sup) in s.supplies.iter().enumerate() {
            let id = format!("{}_{}", s.id, match sup { 'g' => "g", 'y' => "y", _ => "eha" });
            let mut extra = String::new();
            if k == 0 {
                extra += &when("runaway_when", &runaway);
                extra += &when("seal_failed_when", &or(&valve_leak, &seal));
                if supply != "false" {
                    extra += &format!("active_when = \"!({supply})\"\n");
                }
            } else {
                extra += &format!("active_when = \"{supply}\"\n");
            }
            extra += &when("disconnect_when", &disconnect);
            actuator(&mut h, &id, sup, "160 kN", "0.06 m/s", &extra);
            ids.push(format!("\"{id}\""));
        }
        let fault = if sensor == "false" && drift == "false" { String::new() } else { format!("sensor_1_fault = \"if({}, 1, 0)\"\n", or(&sensor, &drift)) };
        writeln!(
            h,
            "[[flight_control.surface]]\nid = \"{}\"\nactuators = [{}]\ncommand = \"{} * {} deg\"\nair = \"air\"\nmin = \"{} deg\"\nmax = \"{} deg\"\ndeflects = \"{}\"\norientation = \"{}\"\narea = {}\n{}{}",
            s.id,
            ids.join(", "),
            s.command,
            s.max,
            s.min,
            s.max,
            s.deflects,
            if s.deflects == "right" { "vertical" } else { "horizontal" },
            s.area,
            when("jam_when", &jam),
            fault,
        )
        .unwrap();
    }

    h += "# Sixteen spoilers on one actuator each: odd panels yellow, even green, panel 6 with an electrical backup (EBHA).\n";
    for side in ["l", "r"] {
        for n in 1..=8 {
            let comp = format!("27_fctl.splr_{side}{n}");
            let fx = |w: &[&str]| f(&comp, w);
            let sup = if n % 2 == 0 { 'g' } else { 'y' };
            let id = format!("splr_{side}{n}");
            let mut extra = String::new();
            if n == 6 {
                extra += "control = \"ebha\"\npowered_when = \"ac_ess.powered\"\n";
            }
            extra += "passive = \"check\"\n";
            extra += &when("runaway_when", &fx(&["hardover"]));
            extra += &when("disconnect_when", &fx(&["disconnect"]));
            extra += &when("seal_failed_when", &or(&fx(&["internal leakage"]), &fx(&["seal wear"])));
            let supply = fx(&["supply loss"]);
            if supply != "false" {
                extra += &format!("active_when = \"!({supply})\"\n");
            }
            actuator(&mut h, &format!("{id}_act"), sup, "80 kN", "0.05 m/s", &extra);
            let roll = if side == "l" { "-roll_cmd.value" } else { "roll_cmd.value" };
            let max = if n <= 2 { 35 } else { 50 };
            writeln!(
                h,
                "[[flight_control.surface]]\nid = \"{id}\"\nactuators = [\"{id}_act\"]\ncommand = \"max(speedbrake_cmd.value, {roll}) * {max} deg\"\nair = \"air\"\nmin = \"0 deg\"\nmax = \"{max} deg\"\ndeflects = \"up\"\narea = 3\n{}",
                when("jam_when", &fx(&["actuator jam"])),
            )
            .unwrap();
        }
    }

    h += "# The stabiliser on a green and a yellow hydraulic motor, with an electric motor on its second input.\n";
    let ths = |w: &[&str]| f("27_fctl.ths", w);
    writeln!(h, "[[actuator.motor]]\nid = \"ths_motor_g\"\ncircuit = \"green\"\n{}", when("jam_when", &ths(&["green motor"]))).unwrap();
    writeln!(h, "[[actuator.motor]]\nid = \"ths_motor_y\"\ncircuit = \"yellow\"\n{}", when("jam_when", &ths(&["yellow motor"]))).unwrap();
    writeln!(
        h,
        "[[flight_control.stabilizer]]\nid = \"ths\"\nmotors = [\"ths_motor_g\", \"ths_motor_y\"]\ncommand = \"ths_cmd.value * 1 deg\"\nair = \"air\"\nelectric_motor = true\nelectric_when = \"green.low && yellow.low && ac_ess.powered\"\n{}{}",
        when("jam_when", &ths(&["ballscrew jam"])),
        when("no_back_failed_when", &ths(&["no-back"])),
    )
    .unwrap();

    h += "# Flaps on two green motors, slats and droop noses on a green motor and an electric one, each drive line with its wing-tip brakes.\n";
    for (id, lever, motors, electric, side_ids) in [("flaps", "flap_lever", ["g", "g"], false, ["flap_l", "flap_r"]), ("slats", "slat_lever", ["g", "g"], true, ["slat_l", "slat_r"]), ("droop", "slat_lever", ["g", "g"], true, ["droop_l", "droop_r"])] {
        let l = |w: &[&str]| f(&format!("27_fctl.{}", side_ids[0]), w);
        let r = |w: &[&str]| f(&format!("27_fctl.{}", side_ids[1]), w);
        let jam = or(&l(&["PCU jam"]), &r(&["PCU jam"]));
        let supply = or(&l(&["supply loss"]), &r(&["supply loss"]));
        for (k, c) in motors.iter().enumerate() {
            let enabled = if supply != "false" { format!("enabled_when = \"!({supply})\"\n") } else { String::new() };
            writeln!(h, "[[actuator.motor]]\nid = \"{id}_motor_{k}\"\ncircuit = \"{}\"\n{}{}", if *c == "y" { "yellow" } else { "green" }, enabled, when("jam_when", &jam)).unwrap();
        }
        writeln!(
            h,
            "[[flight_control.high_lift]]\nid = \"{id}\"\nmotors = [\"{id}_motor_0\", \"{id}_motor_1\"]\ncommand = \"{lever}.value\"\nair = \"air\"\nelectric_motor = {electric}\n{}{}{}{}",
            if electric { "electric_when = \"green.low && ac_ess.powered\"\n" } else { "" },
            when("shaft_break_left_when", &or(&l(&["inboard shaft break"]), &l(&["outboard shaft break"]))),
            when("shaft_break_right_when", &or(&r(&["inboard shaft break"]), &r(&["outboard shaft break"]))),
            when("wtb_when", &or(&l(&["wingtip brake"]), &r(&["wingtip brake"]))),
        )
        .unwrap();
    }

    h += "# Five legs, each on its retraction actuator with its doors; the wing and body legs on bogies, everything on green.\n";
    for (id, cat, mass, retracts, bogie) in [("nose_gear", "nose", 1400, "forward", false), ("wing_gear_l", "l_wing", 5000, "inboard", true), ("wing_gear_r", "r_wing", 5000, "inboard", true), ("body_gear_l", "l_body", 6000, "forward", true), ("body_gear_r", "r_body", 6000, "forward", true)] {
        let comp = format!("32_gear.{cat}_retraction");
        let g = |w: &[&str]| f(&comp, w);
        actuator(&mut h, &format!("{id}_jack"), 'g', "400 kN", "0.12 m/s", &format!("control = \"selector\"\nrod = 0.35\n{}", when("seal_failed_when", &g(&["internal leak"]))));
        actuator(&mut h, &format!("{id}_door_jack"), 'g', "60 kN", "0.2 m/s", &format!("control = \"selector\"\nrod = 0.3\n{}", when("jam_when", &g(&["door"]))));
        writeln!(h, "[[landing_gear.door]]\nid = \"{id}_door\"\nactuator = \"{id}_door_jack\"\nair = \"air\"\n").unwrap();
        let mut bogie_line = String::new();
        if bogie {
            actuator(&mut h, &format!("{id}_trimmer"), 'g', "80 kN", "0.05 m/s", "");
            writeln!(h, "[[landing_gear.bogie]]\nid = \"{id}_bogie\"\nactuator = \"{id}_trimmer\"\non_ground_when = \"{id}.compressed\"\n").unwrap();
            bogie_line = format!("bogie = \"{id}_bogie\"\n");
        }
        let strut = f(&format!("32_gear.{cat}_strut"), &["leak", "gas"]);
        writeln!(
            h,
            "[[landing_gear.leg]]\nid = \"{id}\"\nactuator = \"{id}_jack\"\ndoors = [\"{id}_door\"]\ndown_when = \"gear_lever.on\"\nfree_fall_when = \"gear_gravity.on\"\nretracts = \"{retracts}\"\nmass = \"{mass} kg\"\nair = \"air\"\ninitial_down = true\n{bogie_line}{}{}{}",
            when("uplock_jam_when", &g(&["uplock"])),
            when("downlock_failed_when", &g(&["downlock"])),
            when("strut_leak_when", &strut),
        )
        .unwrap();
    }

    h += "# Sixteen braked wheels: normal brakes on green, alternate on yellow behind its accumulator; parking brake on the alternate line.\n";
    let sides = ["left_wing", "left_wing", "right_wing", "right_wing", "left_wing", "left_wing", "right_wing", "right_wing", "left_body", "left_body", "right_body", "right_body", "left_body", "left_body", "right_body", "right_body"];
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for n in 1..=16 {
        let side = sides[n - 1];
        let k = {
            let c = seen.entry(side).or_insert(0);
            *c += 1;
            *c
        };
        let pedal = if side.starts_with("left") { "brake_left" } else { "brake_right" };
        let comp = format!("32_gear.wheel_{n}_brake");
        let temp = f(&format!("32_gear.brake_temp_{side}_{k}"), &[]);
        let tpis = f(&format!("32_gear.tyre_pressure_{side}_{k}"), &[]);
        writeln!(
            h,
            "[[brakes.wheel]]\nid = \"wheel_{n}\"\ncircuit = \"green\"\nalternate_circuit = \"yellow\"\npedal = \"max({pedal}.value, 0)\"\nparking_when = \"parking_brake.on\"\nnormal_when = \"!green.low\"\nair = \"air\"\n{}{}{}{}",
            when("seized_when", &f(&comp, &["dragging"])),
            when("thermocouple_failed_when", &temp),
            when("tpis_failed_when", &tpis),
            when("tacho_failed_when", &f(&comp, &["antiskid"])),
        )
        .unwrap();
    }

    h += "# Nosewheel steering on two yellow actuators, the body gear's rear axles on green.\n";
    let nws = |w: &[&str]| f("32_gear.nose_steering", w);
    for k in ["a", "b"] {
        actuator(&mut h, &format!("nws_{k}"), 'y', "150 kN", "0.1 m/s", &when("seal_failed_when", &nws(&["internal leak"])));
    }
    writeln!(h, "[[steering.unit]]\nid = \"nose_steering\"\nactuators = [\"nws_a\", \"nws_b\"]\ncommand = \"steer_deg.value * 1 deg\"\nmax = \"70 deg\"\nair = \"air\"\ntowing_when = \"towing.on\"\n{}", when("jam_when", &nws(&["disconnect"]))).unwrap();
    for (id, cat) in [("body_steering_l", "l_body"), ("body_steering_r", "r_body")] {
        actuator(&mut h, &format!("{id}_act"), 'g', "100 kN", "0.08 m/s", "");
        writeln!(h, "[[steering.unit]]\nid = \"{id}\"\nactuators = [\"{id}_act\"]\ncommand = \"-steer_deg.value * 0.2 deg\"\nmax = \"15 deg\"\nopposed = false\nair = \"air\"\ntowing_when = \"towing.on\"\n{}", when("jam_when", &f(&format!("32_gear.{cat}_steering"), &[]))).unwrap();
    }

    h += "[bindings]\n\"sim/joystick/yoke_roll_ratio\" = { to = \"roll_cmd.value\" }\n\"sim/joystick/yoke_pitch_ratio\" = { to = \"pitch_cmd.value\" }\n\"sim/joystick/yoke_heading_ratio\" = { to = \"yaw_cmd.value\" }\n\"sim/cockpit2/controls/speedbrake_ratio\" = { to = \"speedbrake_cmd.value\" }\n\"sim/cockpit2/controls/flap_ratio\" = { to = \"flap_lever.value\" }\n\"sim/cockpit2/controls/left_brake_ratio\" = { to = \"brake_left.value\" }\n\"sim/cockpit2/controls/right_brake_ratio\" = { to = \"brake_right.value\" }\n";
    h
}
