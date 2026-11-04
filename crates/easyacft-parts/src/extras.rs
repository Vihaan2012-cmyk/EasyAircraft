//! Failure triggers, fire protection, lights, sound events and logic building blocks.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const W: Type = Type::Num(Dim::POWER);
const M: Type = Type::Num(Dim::MASS);
const TIME: Type = Type::Num(Dim::TIME);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "failure",
        kind: "trigger",
        help: "Fails a part when a condition holds, after a delay, or at random with a mean time between failures.",
        fields: &[
            req("part", Kind::Ref(&["*"]), "the part to fail"),
            opt("when", WHEN, "false", "for example \"delay(eng1.egt > 1300 K, 10 s)\""),
            opt("latched", Kind::Bool, "true", "stays failed after the condition clears"),
            opt("delay", Kind::Quantity(Dim::TIME), "0 s", "the condition must hold this long"),
            opt("mtbf", Kind::Quantity(Dim::TIME), "0 h", "mean time between failures for random failure while armed; 0 for none"),
            opt("armed_when", WHEN, "true", "random failures count only while this holds, such as in flight"),
            opt("seed", Kind::Number, "1", "random seed, so a run repeats exactly"),
            opt("repair_when", WHEN, "false", "clears a latched failure"),
        ],
        outputs: &[("active", B), ("hazard", N)],
    },
    PartSpec {
        domain: "fire",
        kind: "zone",
        help: "A fire zone: gas temperature from a fire or hot-air leak, two detection loops with their own lag and thresholds, agent concentration that decays with ventilation, re-ignition, damage.",
        fields: &[
            req("fire_when", WHEN, "what sets it alight, for example a failure trigger or \"eng1.egt > 1500 K\""),
            opt("loop_a_failed", WHEN, "false", "loop A open or shorted"),
            opt("loop_b_failed", WHEN, "false", "loop B open or shorted"),
            opt("extinguish_time", Kind::Quantity(Dim::TIME), "3 s", "time the agent must stay at its concentration to put the fire out"),
            opt("overheat_when", WHEN, "false", "a hot-air leak or overheat without flame"),
            opt("volume", Kind::Quantity(Dim::VOLUME), "2 m3", "free volume of the zone"),
            opt("ventilation", Kind::Number, "0.05", "share of the zone's air replaced per second"),
            opt("fire_temperature", Kind::Quantity(Dim::TEMPERATURE), "1100 degC", "gas temperature a fire drives the zone toward"),
            opt("leak_temperature", Kind::Quantity(Dim::TEMPERATURE), "300 degC", "gas temperature a hot-air leak drives it toward"),
            opt("heat_time", Kind::Quantity(Dim::TIME), "3 s", "zone gas heating time constant"),
            opt("loop_time", Kind::Quantity(Dim::TIME), "2 s", "detection loop response time"),
            opt("alarm_temperature", Kind::Quantity(Dim::TEMPERATURE), "400 degC", "loop fire alarm"),
            opt("overheat_temperature", Kind::Quantity(Dim::TEMPERATURE), "200 degC", "loop overheat warning"),
            opt("required_concentration", Kind::Number, "0.06", "agent volume share that suppresses the fire (Halon 1301 about 6 %)"),
            opt("reignite", Kind::Bool, "true", "re-lights when the agent has gone if the source is still there"),
            opt("ambient", Kind::Expr(T), "288 K", "zone temperature with no fire"),
        ],
        outputs: &[("burning", B), ("detected", B), ("loop_fault", B), ("agent", N), ("temperature", T), ("overheat", B), ("loop_a", B), ("loop_b", B), ("damage", N)],
    },
    PartSpec {
        domain: "fire",
        kind: "bottle",
        help: "An extinguisher bottle: agent under nitrogen, pressure with temperature, a squib that needs power, discharge into its zones, thermal discharge overboard.",
        fields: &[
            req("zones", Kind::Refs(&["fire.zone"]), "zones it can reach"),
            req("discharge_when", WHEN, "the squib, for example \"agent1_pb.pressed\""),
            opt("duration", Kind::Quantity(Dim::TIME), "1 s", "time the bottle takes to empty"),
            opt("agent", Kind::Quantity(Dim::MASS), "5 kg", "agent mass"),
            opt("agent_density", Kind::Number, "6.3", "agent vapour density, kg/m3 (Halon 1301 at 20 C)"),
            opt("squib_powered", WHEN, "true", "squib supply, such as its bus"),
            opt("temperature", Kind::Expr(T), "294 K", "bottle temperature"),
            opt("burst_ratio", Kind::Number, "1.8", "pressure over nominal at which the disc bursts"),
            opt("low_ratio", Kind::Number, "0.7", "pressure over nominal counted as low"),
        ],
        outputs: &[("discharged", B), ("pressure", N), ("agent_mass", M), ("low", B), ("thermal_discharge", B), ("failed", B)],
    },
    PartSpec {
        domain: "light",
        kind: "unit",
        help: "A light: steady or flashing, incandescent or LED, dimming with its bus, drawing power, wearing out.",
        fields: &[
            opt("bus", Kind::Ref(&["electrical.bus"]), "", "bus that powers it; none for always powered"),
            opt("on_when", WHEN, "false", "switched on"),
            opt("brightness", Kind::Expr(N), "1", "0 to 1"),
            opt("pattern", Kind::Text, "steady", "steady, beacon, strobe or double"),
            opt("lamp", Kind::Text, "led", "led or incandescent"),
            opt("power", Kind::Quantity(Dim::POWER), "0 W", "rated power, for its load"),
            opt("period", Kind::Quantity(Dim::TIME), "0 s", "flash period; 0 uses the pattern's"),
            opt("flash", Kind::Quantity(Dim::TIME), "0 s", "flash length; 0 uses the pattern's"),
            opt("life", Kind::Quantity(Dim::TIME), "0 h", "lamp life; it burns out past it; 0 for never"),
        ],
        outputs: &[("lit", B), ("brightness", N), ("power", W), ("hours", TIME), ("burnt_out", B), ("failed", B)],
    },
    PartSpec {
        domain: "sound",
        kind: "event",
        help: "A sound cue for FMOD or X-Plane sounds: start and stop pulses, a loop flag, gain with fades and pitch.",
        fields: &[
            opt("trigger", WHEN, "false", "plays on each rising edge"),
            opt("value", Kind::Expr(N), "0", "continuous level, such as a pump's speed share"),
            opt("loop_when", WHEN, "false", "a loop plays while this holds"),
            opt("gain", Kind::Expr(N), "1", "target gain 0 to 1"),
            opt("pitch", Kind::Expr(N), "1", "pitch ratio"),
            opt("fade", Kind::Quantity(Dim::TIME), "0 s", "fade in and out time"),
        ],
        outputs: &[("play", B), ("count", N), ("value", N), ("playing", B), ("start", B), ("stop", B), ("gain", N), ("pitch", N)],
    },
    PartSpec {
        domain: "logic",
        kind: "value",
        help: "A named number from an expression; divide by a unit to get a plain number: \"dc1.voltage / 1 V\".",
        fields: &[req("value", Kind::Expr(N), "an expression giving a plain number")],
        outputs: &[("value", N)],
    },
    PartSpec {
        domain: "logic",
        kind: "filter",
        help: "A first-order lag and rate limit on a plain number, as a sensor or gauge needle responds.",
        fields: &[
            req("input", Kind::Expr(N), "the number to follow"),
            opt("time_constant", Kind::Quantity(Dim::TIME), "1 s", "lag; 0 for none"),
            opt("rate", Kind::Number, "0", "fastest change per second; 0 for no limit"),
            opt("initial", Kind::Number, "0", "value at start"),
        ],
        outputs: &[("value", N)],
    },
    PartSpec {
        domain: "logic",
        kind: "switch",
        help: "A comparator with hysteresis: on above one threshold, off below another, so it does not chatter.",
        fields: &[req("input", Kind::Expr(N), "the number to watch"), req("on_above", Kind::Number, "turns on above this"), req("off_below", Kind::Number, "turns off below this")],
        outputs: &[("on", B)],
    },
    PartSpec {
        domain: "logic",
        kind: "integrator",
        help: "Accumulates a rate over time between limits: a counter, a timer, a level.",
        fields: &[
            req("rate", Kind::Expr(N), "change per second"),
            opt("min", Kind::Number, "-1e12", "lower limit"),
            opt("max", Kind::Number, "1e12", "upper limit"),
            opt("initial", Kind::Number, "0", "value at start and after a reset"),
            opt("reset_when", WHEN, "false", "back to the initial value"),
        ],
        outputs: &[("value", N), ("at_min", B), ("at_max", B)],
    },
];

const HOUR: f64 = 3600.0;

struct Trigger {
    target: usize,
    when: Option<Expr>,
    armed: Option<Expr>,
    repair: Option<Expr>,
    latched: bool,
    delay: f64,
    held: f64,
    mtbf: f64,
    rng: u64,
    next: f64,
    fired: bool,
    s: Slots,
}

/// A small deterministic generator so seeded random failures repeat run to run.
fn next_uniform(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    ((*state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
}

struct Zone {
    e: [Option<Expr>; 5],
    time: f64,
    volume: f64,
    vent: f64,
    fire_t: f64,
    leak_t: f64,
    heat_time: f64,
    loop_time: f64,
    alarm: f64,
    overheat: f64,
    required: f64,
    reignite: bool,
    out: bool,
    burning: bool,
    held: f64,
    gas: f64,
    loops: [f64; 2],
    agent: f64,
    damage: f64,
    s: Slots,
}

struct Bottle {
    zones: Vec<usize>,
    when: Option<Expr>,
    squib: Option<Expr>,
    temp: Option<Expr>,
    duration: f64,
    agent: f64,
    density: f64,
    burst: f64,
    low: f64,
    left: f64,
    used: bool,
    thermal: bool,
    s: Slots,
}

struct Light {
    bus: Option<(usize, f64)>,
    on: Option<Expr>,
    level: Option<Expr>,
    pattern: u8,
    period: f64,
    flash: f64,
    led: bool,
    rated: f64,
    life: f64,
    phase: f64,
    glow: f64,
    hours: f64,
    s: Slots,
}

struct Sound {
    e: [Option<Expr>; 5],
    fade: f64,
    last: bool,
    looping: bool,
    count: f64,
    gain: f64,
    s: Slots,
}

pub struct Extras {
    triggers: Vec<Trigger>,
    zones: Vec<Zone>,
    bottles: Vec<Bottle>,
    lights: Vec<Light>,
    sounds: Vec<Sound>,
    values: Vec<(Option<Expr>, Slots)>,
    filters: Vec<(Option<Expr>, f64, f64, f64, Slots)>,
    switches: Vec<(Option<Expr>, f64, f64, bool, Slots)>,
    integrators: Vec<(Option<Expr>, Option<Expr>, f64, f64, f64, f64, Slots)>,
}

impl Extras {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut x = Extras { triggers: vec![], zones: vec![], bottles: vec![], lights: vec![], sounds: vec![], values: vec![], filters: vec![], switches: vec![], integrators: vec![] };
        let zone_index: Vec<usize> = c.parts.iter().enumerate().filter(|(_, p)| p.domain == "fire" && p.kind == "zone").map(|(i, _)| i).collect();
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("failure", "trigger") => {
                    let Some(t) = p.part("part").and_then(|i| c.slots.index.get(&format!("{}.failed", c.parts[i].id)).copied()) else { continue };
                    let seed = (p.num("seed") as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ p.id.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211));
                    let mut rng = seed.max(1);
                    let mtbf = p.num("mtbf");
                    let next = if mtbf > 0.0 { -mtbf * next_uniform(&mut rng).ln() } else { f64::INFINITY };
                    x.triggers.push(Trigger {
                        target: t,
                        when: expr_of(p, "when"),
                        armed: expr_of(p, "armed_when"),
                        repair: expr_of(p, "repair_when"),
                        latched: p.flag("latched"),
                        delay: p.num("delay").max(0.0),
                        held: 0.0,
                        mtbf,
                        rng,
                        next,
                        fired: false,
                        s: Slots::of(p, &["active", "hazard"]),
                    });
                }
                ("fire", "zone") => x.zones.push(Zone {
                    e: ["fire_when", "loop_a_failed", "loop_b_failed", "overheat_when", "ambient"].map(|k| expr_of(p, k)),
                    time: p.num("extinguish_time").max(0.1),
                    volume: p.num("volume").max(0.01),
                    vent: p.num("ventilation").max(0.0),
                    fire_t: p.num("fire_temperature"),
                    leak_t: p.num("leak_temperature"),
                    heat_time: p.num("heat_time").max(0.1),
                    loop_time: p.num("loop_time").max(0.05),
                    alarm: p.num("alarm_temperature"),
                    overheat: p.num("overheat_temperature"),
                    required: p.num("required_concentration").max(1e-4),
                    reignite: p.flag("reignite"),
                    out: false,
                    burning: false,
                    held: 0.0,
                    gas: 288.15,
                    loops: [288.15; 2],
                    agent: 0.0,
                    damage: 0.0,
                    s: Slots::of(p, &["burning", "detected", "loop_fault", "agent", "temperature", "overheat", "loop_a", "loop_b", "damage"]),
                }),
                ("fire", "bottle") => x.bottles.push(Bottle {
                    zones: p.parts("zones").iter().filter_map(|z| zone_index.iter().position(|i| i == z)).collect(),
                    when: expr_of(p, "discharge_when"),
                    squib: expr_of(p, "squib_powered"),
                    temp: expr_of(p, "temperature"),
                    duration: p.num("duration").max(0.05),
                    agent: p.num("agent").max(0.0),
                    density: p.num("agent_density").max(0.1),
                    burst: p.num("burst_ratio"),
                    low: p.num("low_ratio"),
                    left: p.num("agent").max(0.0),
                    used: false,
                    thermal: false,
                    s: Slots::of(p, &["discharged", "pressure", "agent_mass", "low", "thermal_discharge", "failed"]),
                }),
                ("light", "unit") => {
                    let bus = p.part("bus").and_then(|i| {
                        let b = &c.parts[i];
                        Some((*c.slots.index.get(&format!("{}.voltage", b.id))?, b.num("nominal").max(1.0)))
                    });
                    let pattern = match p.text("pattern") {
                        "beacon" => 1,
                        "strobe" => 2,
                        "double" => 3,
                        _ => 0,
                    };
                    let (dp, df) = match pattern {
                        1 => (1.2, 0.15),
                        2 => (1.0, 0.06),
                        3 => (1.0, 0.05),
                        _ => (1.0, 1.0),
                    };
                    let (period, flash) = (p.num("period"), p.num("flash"));
                    x.lights.push(Light {
                        bus,
                        on: expr_of(p, "on_when"),
                        level: expr_of(p, "brightness"),
                        pattern,
                        period: if period > 0.0 { period } else { dp },
                        flash: if flash > 0.0 { flash } else { df },
                        led: !p.text("lamp").eq_ignore_ascii_case("incandescent"),
                        rated: p.num("power").max(0.0),
                        life: p.num("life").max(0.0),
                        phase: 0.0,
                        glow: 0.0,
                        hours: 0.0,
                        s: Slots::of(p, &["lit", "brightness", "power", "hours", "burnt_out", "failed"]),
                    });
                }
                ("sound", "event") => x.sounds.push(Sound {
                    e: ["trigger", "value", "loop_when", "gain", "pitch"].map(|k| expr_of(p, k)),
                    fade: p.num("fade").max(0.0),
                    last: false,
                    looping: false,
                    count: 0.0,
                    gain: 0.0,
                    s: Slots::of(p, &["play", "count", "value", "playing", "start", "stop", "gain", "pitch"]),
                }),
                ("logic", "value") => x.values.push((expr_of(p, "value"), Slots::of(p, &["value"]))),
                ("logic", "filter") => x.filters.push((expr_of(p, "input"), p.num("time_constant").max(0.0), p.num("rate").max(0.0), p.num("initial"), Slots::of(p, &["value"]))),
                ("logic", "switch") => x.switches.push((expr_of(p, "input"), p.num("on_above"), p.num("off_below"), false, Slots::of(p, &["on"]))),
                ("logic", "integrator") => x.integrators.push((expr_of(p, "rate"), expr_of(p, "reset_when"), p.num("min"), p.num("max"), p.num("initial"), p.num("initial"), Slots::of(p, &["value", "at_min", "at_max"]))),
                _ => {}
            }
        }
        let empty = x.triggers.is_empty() && x.zones.is_empty() && x.bottles.is_empty() && x.lights.is_empty() && x.sounds.is_empty() && x.values.is_empty() && x.filters.is_empty() && x.switches.is_empty() && x.integrators.is_empty();
        (!empty).then_some(x)
    }
}

fn ev(e: &mut Option<Expr>, s: &[f64], dt: f64) -> f64 {
    e.as_mut().map_or(0.0, |x| x.eval(s, dt))
}

fn yes(e: &mut Option<Expr>, s: &[f64], dt: f64) -> bool {
    ev(e, s, dt) != 0.0
}

impl Domain for Extras {
    fn name(&self) -> &'static str {
        "extras"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for t in &mut self.triggers {
            let cond = yes(&mut t.when, slots, dt);
            t.held = if cond { t.held + dt } else { 0.0 };
            let armed = t.armed.as_mut().is_none_or(|e| e.yes(slots, dt));
            let mut random = false;
            if t.mtbf > 0.0 && armed && !t.fired {
                t.next -= dt;
                if t.next <= 0.0 {
                    random = true;
                    t.next = -t.mtbf * next_uniform(&mut t.rng).ln();
                }
            }
            let now = (cond && t.held >= t.delay) || random;
            if yes(&mut t.repair, slots, dt) && t.fired {
                t.fired = false;
                slots[t.target] = 0.0;
            } else if now {
                t.fired = true;
                slots[t.target] = 1.0;
            } else if t.fired && !t.latched && !cond {
                t.fired = false;
                slots[t.target] = 0.0;
            }
            t.s.flag(slots, "active", t.fired);
            t.s.put(slots, "hazard", if t.mtbf > 0.0 { HOUR / t.mtbf } else { 0.0 });
        }
        // Agent from bottles into their zones' volume; it then decays with each zone's ventilation.
        let mut delivered = vec![0.0; self.zones.len()];
        for b in &mut self.bottles {
            let failed = b.s.on(slots, "failed");
            let temp = { let v = ev(&mut b.temp, slots, dt); if v > 100.0 { v } else { 294.0 } };
            let pressure = if b.agent > 0.0 { b.left / b.agent * temp / 294.0 } else { 0.0 };
            if !b.used && !failed && pressure >= b.burst {
                b.thermal = true;
                b.used = true;
            }
            if !b.used && !failed && b.when.as_mut().is_some_and(|e| e.yes(slots, dt)) && b.squib.as_mut().is_none_or(|e| e.yes(slots, dt)) {
                b.used = true;
            }
            if b.used && b.left > 0.0 {
                let out = (b.agent / b.duration * dt).min(b.left);
                b.left -= out;
                if !b.thermal && !b.zones.is_empty() {
                    let share = out / b.density / b.zones.len() as f64;
                    for &z in &b.zones {
                        delivered[z] += share;
                    }
                }
            }
            let p = if b.agent > 0.0 { b.left / b.agent * temp / 294.0 } else { 0.0 };
            b.s.flag(slots, "discharged", b.used);
            b.s.put(slots, "pressure", p);
            b.s.put(slots, "agent_mass", b.left);
            b.s.flag(slots, "low", p < b.low);
            b.s.flag(slots, "thermal_discharge", b.thermal);
        }
        for (k, z) in self.zones.iter_mut().enumerate() {
            let source = yes(&mut z.e[0], slots, dt);
            let fa = yes(&mut z.e[1], slots, dt);
            let fb = yes(&mut z.e[2], slots, dt);
            let leak = yes(&mut z.e[3], slots, dt);
            let ambient = { let v = ev(&mut z.e[4], slots, dt); if v > 100.0 { v } else { 288.15 } };
            z.agent = (z.agent * (1.0 - z.vent * dt).max(0.0) + delivered[k] / z.volume).max(0.0);
            let suppressing = z.agent >= z.required;
            z.held = if suppressing { z.held + dt } else { 0.0 };
            if z.burning && z.held >= z.time {
                z.burning = false;
                z.out = true;
            }
            if !source {
                z.out = false;
            }
            if source && !z.burning && !suppressing && (z.reignite || !z.out) {
                z.burning = true;
            }
            let goal = if z.burning { z.fire_t } else if leak { z.leak_t } else { ambient };
            let tc = if goal > z.gas { z.heat_time } else { (1.0 / z.vent.max(0.01)).min(120.0) };
            z.gas += (goal - z.gas) * (1.0 - (-dt / tc).exp());
            for l in &mut z.loops {
                *l += (z.gas - *l) * (1.0 - (-dt / z.loop_time).exp());
            }
            let alarm = [!fa && z.loops[0] >= z.alarm, !fb && z.loops[1] >= z.alarm];
            let warm = [!fa && z.loops[0] >= z.overheat, !fb && z.loops[1] >= z.overheat];
            // Both healthy loops must agree; one faulty loop leaves the other to decide; both faulty, nothing.
            let vote = |a: [bool; 2]| match (fa, fb) {
                (false, false) => a[0] && a[1],
                (true, false) => a[1],
                (false, true) => a[0],
                _ => false,
            };
            if z.gas > 873.15 {
                z.damage = (z.damage + dt / 300.0).min(1.0);
            }
            z.s.flag(slots, "burning", z.burning);
            z.s.flag(slots, "detected", vote(alarm));
            z.s.flag(slots, "loop_fault", fa || fb);
            z.s.put(slots, "agent", z.agent / z.required);
            z.s.put(slots, "temperature", z.gas);
            z.s.flag(slots, "overheat", vote(warm));
            z.s.flag(slots, "loop_a", alarm[0]);
            z.s.flag(slots, "loop_b", alarm[1]);
            z.s.put(slots, "damage", z.damage);
        }
        for l in &mut self.lights {
            let share = l.bus.map_or(1.0, |(v, nominal)| (slots[v] / nominal).clamp(0.0, 1.2));
            let burnt = l.life > 0.0 && l.hours >= l.life;
            let on = !l.s.on(slots, "failed") && !burnt && l.on.as_mut().is_some_and(|e| e.yes(slots, dt));
            l.phase = (l.phase + dt) % l.period;
            let flash = match l.pattern {
                0 => true,
                3 => l.phase < l.flash || (l.flash * 3.0..l.flash * 4.0).contains(&l.phase),
                _ => l.phase < l.flash,
            };
            // LEDs hold their output down to a driver dropout; filaments follow voltage to the 3.4 and glow up and down.
            let light = if l.led { if share > 0.6 { 1.0 } else { 0.0 } } else { share.powf(3.4) };
            let target = if on && flash { light * ev(&mut l.level, slots, dt).clamp(0.0, 1.0).max(if l.level.is_none() { 1.0 } else { 0.0 }) } else { 0.0 };
            if l.led {
                l.glow = target;
            } else {
                l.glow += (target - l.glow) * (1.0 - (-dt / 0.08).exp());
            }
            if on && flash {
                l.hours += dt;
            }
            let drawn = if on && flash { l.rated * if l.led { share.min(1.0) } else { share.powf(1.6) } } else { 0.0 };
            l.s.flag(slots, "lit", l.glow > 0.05);
            l.s.put(slots, "brightness", l.glow);
            l.s.put(slots, "power", drawn);
            l.s.put(slots, "hours", l.hours);
            l.s.flag(slots, "burnt_out", burnt);
        }
        for s in &mut self.sounds {
            let now = yes(&mut s.e[0], slots, dt);
            let play = now && !s.last;
            s.last = now;
            if play {
                s.count += 1.0;
            }
            let looping = yes(&mut s.e[2], slots, dt);
            let start = (looping && !s.looping) || play;
            let stop = !looping && s.looping;
            s.looping = looping;
            let target = if looping || play { s.e[3].as_mut().map_or(1.0, |e| e.eval(slots, dt)).clamp(0.0, 1.0) } else { 0.0 };
            s.gain = if s.fade > 0.0 { s.gain + (target - s.gain).clamp(-dt / s.fade, dt / s.fade) } else { target };
            let pitch = s.e[4].as_mut().map_or(1.0, |e| e.eval(slots, dt));
            s.s.flag(slots, "play", play);
            s.s.put(slots, "count", s.count);
            s.s.put(slots, "value", ev(&mut s.e[1], slots, dt));
            s.s.flag(slots, "playing", looping || s.gain > 0.0);
            s.s.flag(slots, "start", start);
            s.s.flag(slots, "stop", stop);
            s.s.put(slots, "gain", s.gain);
            s.s.put(slots, "pitch", pitch);
        }
        for (e, s) in &mut self.values {
            let v = ev(e, slots, dt);
            s.put(slots, "value", v);
        }
        for (input, tc, rate, value, s) in &mut self.filters {
            let x = ev(input, slots, dt);
            let mut next = if *tc > 0.0 { *value + (x - *value) * (1.0 - (-dt / *tc).exp()) } else { x };
            if *rate > 0.0 {
                next = *value + (next - *value).clamp(-*rate * dt, *rate * dt);
            }
            *value = next;
            s.put(slots, "value", next);
        }
        for (input, on, off, state, s) in &mut self.switches {
            let x = ev(input, slots, dt);
            if x > *on {
                *state = true;
            } else if x < *off {
                *state = false;
            }
            s.flag(slots, "on", *state);
        }
        for (rate, reset, min, max, initial, value, s) in &mut self.integrators {
            *value = if yes(reset, slots, dt) { *initial } else { (*value + ev(rate, slots, dt) * dt).clamp(*min, *max) };
            s.put(slots, "value", *value);
            s.flag(slots, "at_min", *value <= *min);
            s.flag(slots, "at_max", *value >= *max);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        for t in &self.triggers {
            for v in [t.fired as u8 as f64, t.held, t.next.min(1e15), t.rng as f64] {
                p.put(v);
            }
        }
        for z in &self.zones {
            for v in [z.burning as u8 as f64, z.held, z.gas, z.loops[0], z.loops[1], z.agent, z.damage, z.out as u8 as f64] {
                p.put(v);
            }
        }
        for b in &self.bottles {
            for v in [b.used as u8 as f64, b.left, b.thermal as u8 as f64] {
                p.put(v);
            }
        }
        for l in &self.lights {
            for v in [l.phase, l.glow, l.hours] {
                p.put(v);
            }
        }
        for s in &self.sounds {
            for v in [s.last as u8 as f64, s.count, s.looping as u8 as f64, s.gain] {
                p.put(v);
            }
        }
        self.filters.iter().for_each(|f| p.put(f.3));
        self.switches.iter().for_each(|f| p.put(f.3 as u8 as f64));
        self.integrators.iter().for_each(|f| p.put(f.5));
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        for t in &mut self.triggers {
            t.fired = p.take() != 0.0;
            t.held = p.take();
            t.next = p.take();
            t.rng = (p.take() as u64).max(1);
        }
        for z in &mut self.zones {
            z.burning = p.take() != 0.0;
            z.held = p.take();
            z.gas = p.take();
            z.loops = [p.take(), p.take()];
            z.agent = p.take();
            z.damage = p.take();
            z.out = p.take() != 0.0;
        }
        for b in &mut self.bottles {
            b.used = p.take() != 0.0;
            b.left = p.take();
            b.thermal = p.take() != 0.0;
        }
        for l in &mut self.lights {
            l.phase = p.take();
            l.glow = p.take();
            l.hours = p.take();
        }
        for s in &mut self.sounds {
            s.last = p.take() != 0.0;
            s.count = p.take();
            s.looping = p.take() != 0.0;
            s.gain = p.take();
        }
        self.filters.iter_mut().for_each(|f| f.3 = p.take());
        self.switches.iter_mut().for_each(|f| f.3 = p.take() != 0.0);
        self.integrators.iter_mut().for_each(|f| f.5 = p.take());
    }
}
