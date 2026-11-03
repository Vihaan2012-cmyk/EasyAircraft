//! Computers and data links, air data and inertial sensors, wear counters.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const P: Type = Type::Num(Dim::PRESSURE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const LEN: Type = Type::Num(Dim::LENGTH);
const SPD: Type = Type::Num(Dim::SPEED);
const TIME: Type = Type::Num(Dim::TIME);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const COMPUTER: &[&str] = &["computer.unit"];

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "computer",
        kind: "unit",
        help: "A computer: powered from buses, boots, rides through short power drops, has channels.",
        fields: &[
            opt("power", Kind::Refs(&["electrical.bus"]), "", "buses that power it; any one is enough"),
            opt("powered_when", WHEN, "true", "extra condition, such as its breaker"),
            opt("boot_time", Kind::Quantity(Dim::TIME), "5 s", "power-up and self-test time"),
            opt("hold_up", Kind::Quantity(Dim::TIME), "0.2 s", "power drop it rides through"),
            opt("channels", Kind::Number, "1", "channels; a failed one hands over to the next"),
        ],
        outputs: &[("powered", B), ("booting", B), ("running", B), ("channel", N), ("channel_failed", N), ("frozen", B), ("bad_data", B), ("failed", B)],
    },
    PartSpec {
        domain: "data",
        kind: "link",
        help: "A value one computer sends: rate, latency and status; stale or invalid when the sender stops.",
        fields: &[
            req("from", Kind::Ref(COMPUTER), "sending computer"),
            req("value", Kind::Expr(N), "what it sends, as a plain number"),
            opt("rate", Kind::Quantity(Dim::FREQUENCY), "20 Hz", "update rate"),
            opt("latency", Kind::Quantity(Dim::TIME), "0 s", "transport delay"),
            opt("stale_after", Kind::Quantity(Dim::TIME), "0.5 s", "age at which receivers treat it as lost"),
        ],
        outputs: &[("value", N), ("status", N), ("age", TIME), ("valid", B), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "pitot",
        help: "A pitot probe: total pressure, blocked by ice or a failure.",
        fields: &[req("air", Kind::Ref(AIR), "outside air"), opt("heat_when", WHEN, "false", "probe heat on"), opt("blocked_when", WHEN, "false", "blocked")],
        outputs: &[("pressure", P), ("iced", B), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "static_port",
        help: "A static port.",
        fields: &[req("air", Kind::Ref(AIR), "outside air"), opt("blocked_when", WHEN, "false", "blocked")],
        outputs: &[("pressure", P), ("failed", B)],
    },
    PartSpec {
        domain: "computer",
        kind: "air_data",
        help: "An air data computer working from its probes.",
        fields: &[
            req("pitot", Kind::Ref(&["sensor.pitot"]), "pitot probe"),
            req("static", Kind::Ref(&["sensor.static_port"]), "static port"),
            req("air", Kind::Ref(AIR), "outside air, for total air temperature"),
            opt("computer", Kind::Ref(COMPUTER), "", "the computer it runs on, for power"),
            opt("qnh", Kind::Expr(P), "1013.25 hPa", "altimeter setting"),
        ],
        outputs: &[("ias", SPD), ("mach", N), ("altitude", LEN), ("vertical_speed", SPD), ("tat", T), ("sat", T), ("valid", B), ("failed", B)],
    },
    PartSpec {
        domain: "computer",
        kind: "inertial",
        help: "An inertial reference unit: aligns, then drifts.",
        fields: &[
            req("air", Kind::Ref(AIR), "flight state: heading, pitch, roll, position"),
            opt("computer", Kind::Ref(COMPUTER), "", "the computer it runs on, for power"),
            opt("mode", Kind::Expr(N), "1", "0 off, 1 navigation, 2 attitude only"),
            opt("align_time", Kind::Quantity(Dim::TIME), "600 s", "alignment time"),
            opt("drift", Kind::Number, "1", "position drift, nautical miles per hour"),
        ],
        outputs: &[("aligning", B), ("aligned", B), ("attitude_valid", B), ("nav_valid", B), ("heading", N), ("pitch", N), ("roll", N), ("position_error", LEN), ("failed", B)],
    },
    PartSpec {
        domain: "wear",
        kind: "counter",
        help: "Wear or a consumable kept between flights: brakes, tyres, oil, filters.",
        fields: &[
            opt("initial", Kind::Number, "0", "value on a new aircraft"),
            req("rate", Kind::Expr(N), "change per second, for example \"if(brakes.pressure > 0 Pa, 0.0001, 0)\""),
            opt("limit", Kind::Number, "1", "value at which it counts as worn"),
            opt("service_when", WHEN, "false", "servicing resets it"),
            opt("serviced", Kind::Number, "0", "value after servicing"),
        ],
        outputs: &[("value", N), ("worn", B)],
    },
];

struct Computer {
    power: Vec<usize>,
    when: Option<Expr>,
    boot: f64,
    hold: f64,
    channels: f64,
    booted: f64,
    unpowered: f64,
    s: Slots,
}

struct Link {
    from: usize,
    frozen: usize,
    bad: usize,
    value: Option<Expr>,
    period: f64,
    latency: f64,
    stale: f64,
    since_send: f64,
    queue: Vec<(f64, f64)>,
    held: f64,
    age: f64,
    s: Slots,
}

struct Probe {
    air: [usize; 4],
    heat: Option<Expr>,
    blocked: Option<Expr>,
    pitot: bool,
    held: f64,
    s: Slots,
}

struct Adc {
    pitot: usize,
    stat: usize,
    tat_air: [usize; 2],
    running: Option<usize>,
    qnh: Option<Expr>,
    last_alt: Option<f64>,
    s: Slots,
}

struct Irs {
    air: [usize; 5],
    running: Option<usize>,
    mode: Option<Expr>,
    align: f64,
    drift: f64,
    t: f64,
    nav_t: f64,
    s: Slots,
}

pub struct Avionics {
    computers: Vec<Computer>,
    links: Vec<Link>,
    probes: Vec<Probe>,
    adcs: Vec<Adc>,
    irs: Vec<Irs>,
    wear: Vec<(Option<Expr>, Option<Expr>, f64, f64, f64, Slots)>,
}

fn s_of(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "wear" && p.kind == "counter") {
        slots[crate::catalog::out(p, "value")] = p.num("initial");
    }
}

impl Avionics {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut a = Avionics { computers: vec![], links: vec![], probes: vec![], adcs: vec![], irs: vec![], wear: vec![] };
        let air3 = |p: &CheckedPart| -> Option<[usize; 4]> { Some([s_of(c, p, "air", "pressure")?, s_of(c, p, "air", "airspeed")?, s_of(c, p, "air", "icing")?, s_of(c, p, "air", "temperature")?]) };
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("computer", "unit") => a.computers.push(Computer {
                    power: p.parts("power").iter().filter_map(|&i| c.slots.index.get(&format!("{}.powered", c.parts[i].id)).copied()).collect(),
                    when: expr_of(p, "powered_when"),
                    boot: p.num("boot_time"),
                    hold: p.num("hold_up"),
                    channels: p.num("channels").max(1.0),
                    booted: 0.0,
                    unpowered: 1e9,
                    s: Slots::of(p, &["powered", "booting", "running", "channel", "channel_failed", "frozen", "bad_data", "failed"]),
                }),
                ("data", "link") => {
                    let (Some(from), Some(frozen), Some(bad)) = (s_of(c, p, "from", "running"), s_of(c, p, "from", "frozen"), s_of(c, p, "from", "bad_data")) else { continue };
                    a.links.push(Link {
                        from,
                        frozen,
                        bad,
                        value: expr_of(p, "value"),
                        period: 1.0 / p.num("rate").max(0.01),
                        latency: p.num("latency"),
                        stale: p.num("stale_after"),
                        since_send: 0.0,
                        queue: Vec::new(),
                        held: 0.0,
                        age: 1e9,
                        s: Slots::of(p, &["value", "status", "age", "valid", "failed"]),
                    });
                }
                ("sensor", "pitot") | ("sensor", "static_port") => {
                    let Some(air) = air3(p) else { continue };
                    let pitot = p.kind == "pitot";
                    a.probes.push(Probe {
                        air,
                        heat: if pitot { expr_of(p, "heat_when") } else { None },
                        blocked: expr_of(p, "blocked_when"),
                        pitot,
                        held: 101_325.0,
                        s: Slots::of(p, if pitot { &["pressure", "iced", "failed"] } else { &["pressure", "failed"] }),
                    });
                }
                ("computer", "air_data") => {
                    let (Some(pt), Some(ps), Some(t), Some(m)) = (s_of(c, p, "pitot", "pressure"), s_of(c, p, "static", "pressure"), s_of(c, p, "air", "temperature"), s_of(c, p, "air", "mach")) else { continue };
                    a.adcs.push(Adc { pitot: pt, stat: ps, tat_air: [t, m], running: s_of(c, p, "computer", "running"), qnh: expr_of(p, "qnh"), last_alt: None, s: Slots::of(p, &["ias", "mach", "altitude", "vertical_speed", "tat", "sat", "valid", "failed"]) });
                }
                ("computer", "inertial") => {
                    let ids = ["heading", "pitch", "roll", "groundspeed", "on_ground"];
                    let Some(air) = ids.iter().map(|n| s_of(c, p, "air", n)).collect::<Option<Vec<_>>>() else { continue };
                    a.irs.push(Irs {
                        air: [air[0], air[1], air[2], air[3], air[4]],
                        running: s_of(c, p, "computer", "running"),
                        mode: expr_of(p, "mode"),
                        align: p.num("align_time").max(1.0),
                        drift: p.num("drift") * 1852.0 / 3600.0,
                        t: 0.0,
                        nav_t: 0.0,
                        s: Slots::of(p, &["aligning", "aligned", "attitude_valid", "nav_valid", "heading", "pitch", "roll", "position_error", "failed"]),
                    });
                }
                ("wear", "counter") => a.wear.push((expr_of(p, "rate"), expr_of(p, "service_when"), p.num("limit"), p.num("serviced"), p.num("initial"), Slots::of(p, &["value", "worn"]))),
                _ => {}
            }
        }
        let empty = a.computers.is_empty() && a.links.is_empty() && a.probes.is_empty() && a.adcs.is_empty() && a.irs.is_empty() && a.wear.is_empty();
        (!empty).then_some(a)
    }
}

const RHO0: f64 = 1.225;
const P0: f64 = 101_325.0;

fn ias_from_qc(qc: f64) -> f64 {
    let x = (qc.max(0.0) / P0 + 1.0).powf(0.4 / 1.4) - 1.0;
    (2.0 * 1.4 / 0.4 * P0 / RHO0 * x).sqrt()
}

impl Domain for Avionics {
    fn name(&self) -> &'static str {
        "avionics"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for c in &mut self.computers {
            let failed = c.s.on(slots, "failed");
            let bus = c.power.is_empty() || c.power.iter().any(|&b| slots[b] != 0.0);
            let powered = bus && c.when.as_mut().is_none_or(|e| e.yes(slots, dt)) && !failed;
            c.unpowered = if powered { 0.0 } else { c.unpowered + dt };
            if c.unpowered > c.hold {
                c.booted = 0.0;
            } else if powered {
                c.booted += dt;
            }
            let lost = c.s.get(slots, "channel_failed").max(0.0);
            let channel = if lost >= c.channels { 0.0 } else { lost.floor() + 1.0 };
            let running = c.unpowered <= c.hold && c.booted >= c.boot && channel > 0.0;
            c.s.flag(slots, "powered", powered);
            c.s.flag(slots, "booting", powered && c.booted < c.boot);
            c.s.flag(slots, "running", running);
            c.s.put(slots, "channel", channel);
        }
        for l in &mut self.links {
            let up = slots[l.from] != 0.0 && !l.s.on(slots, "failed");
            let v = l.value.as_mut().map_or(0.0, |e| e.eval(slots, dt));
            l.since_send += dt;
            if up && slots[l.frozen] == 0.0 && l.since_send >= l.period {
                l.since_send = 0.0;
                let sent = if slots[l.bad] != 0.0 { v * 1.37 + 11.0 } else { v };
                l.queue.push((l.latency, sent));
            }
            for q in &mut l.queue {
                q.0 -= dt;
            }
            while l.queue.first().is_some_and(|q| q.0 <= 0.0) {
                l.held = l.queue.remove(0).1;
                l.age = 0.0;
            }
            l.age += dt;
            let valid = up && l.age <= l.stale;
            let status = if valid { 0.0 } else if up { 1.0 } else { 2.0 };
            l.s.put(slots, "value", l.held);
            l.s.put(slots, "status", status);
            l.s.put(slots, "age", l.age.min(1e6));
            l.s.flag(slots, "valid", valid);
        }
        for pr in &mut self.probes {
            let (p, v, icing, t) = (slots[pr.air[0]], slots[pr.air[1]], slots[pr.air[2]], slots[pr.air[3]].max(150.0));
            let heated = pr.heat.as_mut().is_some_and(|e| e.yes(slots, dt));
            let iced = pr.pitot && icing > 0.3 && t < 275.15 && !heated;
            let blocked = iced || pr.s.on(slots, "failed") || pr.blocked.as_mut().is_some_and(|e| e.yes(slots, dt));
            if !blocked {
                let m = v / (1.4 * 287.05 * t).sqrt();
                pr.held = if pr.pitot { p * (1.0 + 0.2 * m * m).powf(3.5) } else { p };
            }
            pr.s.put(slots, "pressure", pr.held);
            if pr.pitot {
                pr.s.flag(slots, "iced", iced);
            }
        }
        for a in &mut self.adcs {
            let ok = a.running.is_none_or(|r| slots[r] != 0.0) && !a.s.on(slots, "failed");
            let (pt, ps) = (slots[a.pitot], slots[a.stat].max(1.0));
            let qnh = a.qnh.as_mut().map_or(P0, |e| e.eval(slots, dt)).max(1.0);
            let mach = ((2.0 / 0.4) * ((pt.max(ps) / ps).powf(0.4 / 1.4) - 1.0)).max(0.0).sqrt();
            let alt = 44_330.8 * (1.0 - (ps / qnh).powf(0.190_263));
            let vs = a.last_alt.map_or(0.0, |l| (alt - l) / dt.max(1e-6));
            a.last_alt = Some(alt);
            let tat = slots[a.tat_air[0]] * (1.0 + 0.2 * slots[a.tat_air[1]].powi(2));
            let sat = tat / (1.0 + 0.2 * mach * mach);
            let vals = [ias_from_qc(pt - ps), mach, alt, vs, tat, sat];
            for (name, v) in ["ias", "mach", "altitude", "vertical_speed", "tat", "sat"].iter().zip(vals) {
                a.s.put(slots, name, if ok { v } else { 0.0 });
            }
            a.s.flag(slots, "valid", ok);
        }
        for i in &mut self.irs {
            let ok = i.running.is_none_or(|r| slots[r] != 0.0) && !i.s.on(slots, "failed");
            let mode = i.mode.as_mut().map_or(1.0, |e| e.eval(slots, dt)).round();
            let still = slots[i.air[4]] != 0.0 && slots[i.air[3]].abs() < 0.5;
            if !ok || mode == 0.0 {
                i.t = 0.0;
                i.nav_t = 0.0;
            } else if i.t < i.align {
                if still || mode == 2.0 {
                    i.t += dt;
                }
            } else {
                i.nav_t += dt;
            }
            let aligned = ok && i.t >= i.align && mode == 1.0;
            let att = ok && (aligned || (mode == 2.0 && i.t >= 30.0));
            i.s.flag(slots, "aligning", ok && mode != 0.0 && i.t < i.align);
            i.s.flag(slots, "aligned", aligned);
            i.s.flag(slots, "attitude_valid", att);
            i.s.flag(slots, "nav_valid", aligned);
            for (k, name) in ["heading", "pitch", "roll"].iter().enumerate() {
                i.s.put(slots, name, if att { slots[i.air[k]] } else { 0.0 });
            }
            i.s.put(slots, "position_error", if aligned { i.drift * i.nav_t } else { 0.0 });
        }
        for (rate, service, limit, serviced, _, s) in &mut self.wear {
            let mut v = s.get(slots, "value") + rate.as_mut().map_or(0.0, |e| e.eval(slots, dt)) * dt;
            if service.as_mut().is_some_and(|e| e.yes(slots, dt)) {
                v = *serviced;
            }
            s.put(slots, "value", v);
            s.flag(slots, "worn", v >= *limit);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        for c in &self.computers {
            p.put(c.booted);
            p.put(c.unpowered.min(1e9));
        }
        for l in &self.links {
            p.put(l.held);
            p.put(l.age.min(1e9));
            p.put(l.since_send);
        }
        self.probes.iter().for_each(|x| p.put(x.held));
        for i in &self.irs {
            p.put(i.t);
            p.put(i.nav_t);
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        for c in &mut self.computers {
            c.booted = p.take();
            c.unpowered = p.take();
        }
        for l in &mut self.links {
            l.held = p.take();
            l.age = p.take();
            l.since_send = p.take();
            l.queue.clear();
        }
        self.probes.iter_mut().for_each(|x| x.held = p.take());
        for i in &mut self.irs {
            i.t = p.take();
            i.nav_t = p.take();
        }
    }
}
