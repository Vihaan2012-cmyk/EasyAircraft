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
const W: Type = Type::Num(Dim::POWER);
const WHEN: Kind = Kind::Expr(Type::Bool);
const AIR: &[&str] = &["environment.air"];
const COMPUTER: &[&str] = &["computer.unit"];

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "computer",
        kind: "unit",
        help: "A computer: powered from buses, boots and self-tests, rides through short power drops, has channels that hand over, overheats without cooling, draws power.",
        fields: &[
            opt("power", Kind::Refs(&["electrical.bus"]), "", "buses that power it; any one is enough"),
            opt("powered_when", WHEN, "true", "extra condition, such as its breaker"),
            opt("boot_time", Kind::Quantity(Dim::TIME), "5 s", "power-up and self-test time"),
            opt("hold_up", Kind::Quantity(Dim::TIME), "0.2 s", "power drop it rides through"),
            opt("channels", Kind::Number, "1", "channels; a failed one hands over to the next"),
            opt("transfer_time", Kind::Quantity(Dim::TIME), "0 s", "time output is lost while a channel hands over"),
            opt("load", Kind::Quantity(Dim::POWER), "0 W", "power it draws running; twice that while booting"),
            opt("cooled_when", WHEN, "true", "its cooling air or fan is working"),
            opt("overheat_time", Kind::Quantity(Dim::TIME), "0 s", "time without cooling before it shuts itself down; 0 for never"),
            opt("bite_failed_when", WHEN, "false", "its self-test finds a fault: it stays in boot"),
            opt("reset_when", WHEN, "false", "a watchdog or reset: it reboots"),
        ],
        outputs: &[
            ("powered", B),
            ("booting", B),
            ("running", B),
            ("channel", N),
            ("channel_failed", N),
            ("frozen", B),
            ("bad_data", B),
            ("failed", B),
            ("degraded", B),
            ("transferring", B),
            ("temperature", T),
            ("overheat", B),
            ("load", W),
        ],
    },
    PartSpec {
        domain: "data",
        kind: "link",
        help: "A value one computer sends, as an ARINC 429 word: rate, latency, range and resolution, sign/status matrix; stale or invalid when the sender stops.",
        fields: &[
            req("from", Kind::Ref(COMPUTER), "sending computer"),
            opt("to", Kind::Refs(COMPUTER), "", "receiving computers, for the network picture; reading the link is what uses it"),
            req("value", Kind::Expr(N), "what it sends, as a plain number"),
            opt("rate", Kind::Quantity(Dim::FREQUENCY), "20 Hz", "update rate"),
            opt("latency", Kind::Quantity(Dim::TIME), "0 s", "transport delay"),
            opt("stale_after", Kind::Quantity(Dim::TIME), "0.5 s", "age at which receivers treat it as lost"),
            opt("label", Kind::Number, "0", "label, octal as written (for example 203 for altitude)"),
            opt("sdi", Kind::Number, "0", "source/destination identifier 0 to 3"),
            opt("range", Kind::Number, "0", "BNR full scale: values beyond it clip; 0 sends the value unchanged"),
            opt("bits", Kind::Number, "18", "BNR significant bits: the resolution is range / 2^bits"),
            opt("valid_when", WHEN, "true", "the sender has computed data; otherwise no computed data (NCD)"),
            opt("test_when", WHEN, "false", "functional test (FT)"),
        ],
        outputs: &[("value", N), ("status", N), ("age", TIME), ("valid", B), ("ssm", N), ("word", N), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "pitot",
        help: "A pitot probe: total pressure through a pneumatic lag, heated against ice, blocked by ice or a failure.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            opt("heat_when", WHEN, "false", "probe heat on"),
            opt("blocked_when", WHEN, "false", "blocked"),
            opt("heater", Kind::Quantity(Dim::POWER), "200 W", "heater power"),
            opt("block_time", Kind::Quantity(Dim::TIME), "30 s", "unheated in full icing, time until ice closes the inlet"),
            opt("lag", Kind::Quantity(Dim::TIME), "0.1 s", "line lag"),
        ],
        outputs: &[("pressure", P), ("iced", B), ("temperature", T), ("blockage", N), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "static_port",
        help: "A static port: static pressure with its position error, which grows with sideslip.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            opt("blocked_when", WHEN, "false", "blocked"),
            opt("error", Kind::Number, "0", "position error, share of dynamic pressure"),
            opt("sideslip_error", Kind::Number, "0", "added error per radian of sideslip, share of dynamic pressure"),
            opt("lag", Kind::Quantity(Dim::TIME), "0.2 s", "line lag"),
        ],
        outputs: &[("pressure", P), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "tat",
        help: "A total air temperature probe: recovery, heater error at low speed, lag.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            opt("recovery", Kind::Number, "0.99", "share of the kinetic temperature rise it recovers"),
            opt("heat_when", WHEN, "false", "probe heat on; it reads high at low airspeed"),
            opt("lag", Kind::Quantity(Dim::TIME), "2 s", "time constant"),
        ],
        outputs: &[("temperature", T), ("failed", B)],
    },
    PartSpec {
        domain: "sensor",
        kind: "aoa",
        help: "An angle-of-attack vane: lag, ice can freeze it, a failure holds it.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            opt("heat_when", WHEN, "false", "vane heat on"),
            opt("lag", Kind::Quantity(Dim::TIME), "0.15 s", "vane response"),
            opt("freeze_time", Kind::Quantity(Dim::TIME), "120 s", "unheated in full icing, time until it freezes in place"),
        ],
        outputs: &[("angle", N), ("frozen", B), ("failed", B)],
    },
    PartSpec {
        domain: "computer",
        kind: "air_data",
        help: "An air data computer working from its probes: CAS, Mach, TAS, altitudes, filtered vertical speed, temperatures, overspeed.",
        fields: &[
            req("pitot", Kind::Ref(&["sensor.pitot"]), "pitot probe"),
            req("static", Kind::Ref(&["sensor.static_port"]), "static port"),
            req("air", Kind::Ref(AIR), "outside air, for total air temperature"),
            opt("computer", Kind::Ref(COMPUTER), "", "the computer it runs on, for power"),
            opt("qnh", Kind::Expr(P), "1013.25 hPa", "altimeter setting"),
            opt("tat_probe", Kind::Ref(&["sensor.tat"]), "", "its own TAT probe; otherwise the air's"),
            opt("vmo", Kind::Quantity(Dim::SPEED), "0 kt", "maximum operating speed; 0 for no overspeed warning"),
            opt("mmo", Kind::Number, "0", "maximum operating Mach; 0 for none"),
            opt("vs_lag", Kind::Quantity(Dim::TIME), "1 s", "vertical speed filter"),
        ],
        outputs: &[
            ("ias", SPD),
            ("mach", N),
            ("altitude", LEN),
            ("vertical_speed", SPD),
            ("tat", T),
            ("sat", T),
            ("valid", B),
            ("failed", B),
            ("cas", SPD),
            ("tas", SPD),
            ("pressure_altitude", LEN),
            ("overspeed", B),
        ],
    },
    PartSpec {
        domain: "computer",
        kind: "inertial",
        help: "An inertial reference unit: aligns (longer at high latitude, restarting if moved), then drifts with a Schuler oscillation.",
        fields: &[
            req("air", Kind::Ref(AIR), "flight state: heading, pitch, roll, position"),
            opt("computer", Kind::Ref(COMPUTER), "", "the computer it runs on, for power"),
            opt("mode", Kind::Expr(N), "1", "0 off, 1 navigation, 2 attitude only"),
            opt("align_time", Kind::Quantity(Dim::TIME), "600 s", "alignment time up to 60 degrees latitude; 1.7 times that by 78"),
            opt("drift", Kind::Number, "1", "position drift, nautical miles per hour"),
            opt("heading_drift", Kind::Number, "0", "heading drift in attitude mode, degrees per hour"),
            opt("position_entered", WHEN, "true", "the crew has entered present position; alignment finishes only then"),
            opt("restart_on_motion", Kind::Bool, "true", "moving during alignment starts it again"),
        ],
        outputs: &[
            ("aligning", B),
            ("aligned", B),
            ("attitude_valid", B),
            ("nav_valid", B),
            ("heading", N),
            ("pitch", N),
            ("roll", N),
            ("position_error", LEN),
            ("failed", B),
            ("align_remaining", TIME),
            ("latitude", N),
            ("longitude", N),
        ],
    },
    PartSpec {
        domain: "wear",
        kind: "counter",
        help: "Wear or a consumable kept between flights: brakes, tyres, oil, filters; cycles, life and failure probability.",
        fields: &[
            opt("initial", Kind::Number, "0", "value on a new aircraft"),
            opt("rate", Kind::Expr(N), "0", "change per second, for example \"if(brakes.pressure > 0 Pa, 0.0001, 0)\""),
            opt("limit", Kind::Number, "1", "value at which it counts as worn"),
            opt("service_when", WHEN, "false", "servicing resets it"),
            opt("serviced", Kind::Number, "0", "value after servicing"),
            opt("cycle_when", WHEN, "false", "each rising edge is a cycle, such as a landing"),
            opt("per_cycle", Kind::Number, "0", "added per cycle"),
            opt("life", Kind::Number, "0", "characteristic life for failure probability, in the counter's units; 0 for none"),
            opt("shape", Kind::Number, "2", "Weibull shape: above 1 for wear-out"),
        ],
        outputs: &[("value", N), ("worn", B), ("cycles", N), ("failure_probability", N)],
    },
];

struct Computer {
    power: Vec<usize>,
    e: [Option<Expr>; 4],
    boot: f64,
    hold: f64,
    channels: f64,
    transfer: f64,
    load: f64,
    overheat_t: f64,
    booted: f64,
    unpowered: f64,
    last_channel: f64,
    transferring: f64,
    hot: f64,
    s: Slots,
}

struct Link {
    from: usize,
    frozen: usize,
    bad: usize,
    value: Option<Expr>,
    valid_e: Option<Expr>,
    test_e: Option<Expr>,
    period: f64,
    latency: f64,
    stale: f64,
    label: u32,
    sdi: u32,
    range: f64,
    bits: u32,
    since_send: f64,
    queue: Vec<(f64, f64, f64)>,
    held: f64,
    ssm: f64,
    age: f64,
    s: Slots,
}

struct Probe {
    air: [usize; 4],
    beta: Option<usize>,
    heat: Option<Expr>,
    blocked: Option<Expr>,
    pitot: bool,
    heater: f64,
    block_time: f64,
    lag: f64,
    error: f64,
    beta_error: f64,
    held: f64,
    temp: f64,
    blockage: f64,
    s: Slots,
}

struct Tat {
    air: [usize; 3],
    heat: Option<Expr>,
    recovery: f64,
    lag: f64,
    t: f64,
    s: Slots,
}

struct Vane {
    air: [usize; 3],
    heat: Option<Expr>,
    lag: f64,
    freeze: f64,
    angle: f64,
    ice: f64,
    s: Slots,
}

struct Adc {
    pitot: usize,
    stat: usize,
    tat_air: [usize; 2],
    tat_probe: Option<usize>,
    running: Option<usize>,
    qnh: Option<Expr>,
    vmo: f64,
    mmo: f64,
    vs_lag: f64,
    last_alt: Option<f64>,
    vs: f64,
    s: Slots,
}

struct Irs {
    air: [usize; 7],
    running: Option<usize>,
    mode: Option<Expr>,
    entered: Option<Expr>,
    align: f64,
    drift: f64,
    heading_drift: f64,
    restart: bool,
    t: f64,
    nav_t: f64,
    att_t: f64,
    s: Slots,
}

struct Wear {
    rate: Option<Expr>,
    service: Option<Expr>,
    cycle: Option<Expr>,
    limit: f64,
    serviced: f64,
    per_cycle: f64,
    life: f64,
    shape: f64,
    last: bool,
    cycles: f64,
    s: Slots,
}

pub struct Avionics {
    computers: Vec<Computer>,
    links: Vec<Link>,
    probes: Vec<Probe>,
    tats: Vec<Tat>,
    vanes: Vec<Vane>,
    adcs: Vec<Adc>,
    irs: Vec<Irs>,
    wear: Vec<Wear>,
}

fn s_of(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

pub fn init(c: &Checked, slots: &mut [f64]) {
    for p in c.parts.iter().filter(|p| p.domain == "wear" && p.kind == "counter") {
        slots[crate::catalog::out(p, "value")] = p.num("initial");
    }
    for p in c.parts.iter().filter(|p| p.domain == "computer" && p.kind == "unit") {
        slots[crate::catalog::out(p, "temperature")] = 298.15;
    }
}

/// Octal label digits as written ("203") to their value.
fn octal(n: f64) -> u32 {
    let digits = format!("{}", n.max(0.0) as u64);
    u32::from_str_radix(&digits, 8).unwrap_or(0) & 0xFF
}

/// An ARINC 429 BNR word: label in bits 1-8 sent reversed, SDI 9-10, two's-complement data 11-29, SSM 30-31, odd parity 32.
fn word(label: u32, sdi: u32, ssm: u32, value: f64, range: f64, bits: u32) -> f64 {
    let data = if range > 0.0 {
        let lsb = range / (1u64 << bits.min(19)) as f64;
        let counts = (value.clamp(-range, range - lsb) / lsb).round() as i64;
        ((counts << (19 - bits.min(19))) as u32) & 0x7FFFF
    } else {
        0
    };
    let mut w = label.reverse_bits() >> 24 | (sdi & 3) << 8 | data << 10 | (ssm & 3) << 29;
    if w.count_ones() % 2 == 0 {
        w |= 1 << 31;
    }
    w as f64
}

impl Avionics {
    pub fn new(c: &Checked) -> Option<Self> {
        let mut a = Avionics { computers: vec![], links: vec![], probes: vec![], tats: vec![], vanes: vec![], adcs: vec![], irs: vec![], wear: vec![] };
        let air4 = |p: &CheckedPart| -> Option<[usize; 4]> { Some([s_of(c, p, "air", "pressure")?, s_of(c, p, "air", "airspeed")?, s_of(c, p, "air", "icing")?, s_of(c, p, "air", "temperature")?]) };
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("computer", "unit") => a.computers.push(Computer {
                    power: p.parts("power").iter().filter_map(|&i| c.slots.index.get(&format!("{}.powered", c.parts[i].id)).copied()).collect(),
                    e: ["powered_when", "cooled_when", "bite_failed_when", "reset_when"].map(|k| expr_of(p, k)),
                    boot: p.num("boot_time"),
                    hold: p.num("hold_up"),
                    channels: p.num("channels").max(1.0),
                    transfer: p.num("transfer_time").max(0.0),
                    load: p.num("load").max(0.0),
                    overheat_t: p.num("overheat_time").max(0.0),
                    booted: 0.0,
                    unpowered: 1e9,
                    last_channel: 1.0,
                    transferring: 0.0,
                    hot: 0.0,
                    s: Slots::of(p, &["powered", "booting", "running", "channel", "channel_failed", "frozen", "bad_data", "failed", "degraded", "transferring", "temperature", "overheat", "load"]),
                }),
                ("data", "link") => {
                    let (Some(from), Some(frozen), Some(bad)) = (s_of(c, p, "from", "running"), s_of(c, p, "from", "frozen"), s_of(c, p, "from", "bad_data")) else { continue };
                    a.links.push(Link {
                        from,
                        frozen,
                        bad,
                        value: expr_of(p, "value"),
                        valid_e: expr_of(p, "valid_when"),
                        test_e: expr_of(p, "test_when"),
                        period: 1.0 / p.num("rate").max(0.01),
                        latency: p.num("latency"),
                        stale: p.num("stale_after"),
                        label: octal(p.num("label")),
                        sdi: p.num("sdi").clamp(0.0, 3.0) as u32,
                        range: p.num("range").max(0.0),
                        bits: p.num("bits").clamp(1.0, 19.0) as u32,
                        since_send: 0.0,
                        queue: Vec::new(),
                        held: 0.0,
                        ssm: 0.0,
                        age: 1e9,
                        s: Slots::of(p, &["value", "status", "age", "valid", "ssm", "word", "failed"]),
                    });
                }
                ("sensor", "pitot") | ("sensor", "static_port") => {
                    let Some(air) = air4(p) else { continue };
                    let pitot = p.kind == "pitot";
                    a.probes.push(Probe {
                        air,
                        beta: s_of(c, p, "air", "sideslip"),
                        heat: if pitot { expr_of(p, "heat_when") } else { None },
                        blocked: expr_of(p, "blocked_when"),
                        pitot,
                        heater: if pitot { p.num("heater").max(0.0) } else { 0.0 },
                        block_time: if pitot { p.num("block_time").max(1.0) } else { 1.0 },
                        lag: p.num("lag").max(0.0),
                        error: if pitot { 0.0 } else { p.num("error") },
                        beta_error: if pitot { 0.0 } else { p.num("sideslip_error") },
                        held: 101_325.0,
                        temp: 288.15,
                        blockage: 0.0,
                        s: Slots::of(p, if pitot { &["pressure", "iced", "temperature", "blockage", "failed"] } else { &["pressure", "failed"] }),
                    });
                }
                ("sensor", "tat") => {
                    let (Some(t), Some(m), Some(v)) = (s_of(c, p, "air", "temperature"), s_of(c, p, "air", "mach"), s_of(c, p, "air", "airspeed")) else { continue };
                    a.tats.push(Tat { air: [t, m, v], heat: expr_of(p, "heat_when"), recovery: p.num("recovery"), lag: p.num("lag").max(0.01), t: 288.15, s: Slots::of(p, &["temperature", "failed"]) });
                }
                ("sensor", "aoa") => {
                    let (Some(aoa), Some(icing), Some(t)) = (s_of(c, p, "air", "aoa"), s_of(c, p, "air", "icing"), s_of(c, p, "air", "temperature")) else { continue };
                    a.vanes.push(Vane { air: [aoa, icing, t], heat: expr_of(p, "heat_when"), lag: p.num("lag").max(0.01), freeze: p.num("freeze_time").max(1.0), angle: 0.0, ice: 0.0, s: Slots::of(p, &["angle", "frozen", "failed"]) });
                }
                ("computer", "air_data") => {
                    let (Some(pt), Some(ps), Some(t), Some(m)) = (s_of(c, p, "pitot", "pressure"), s_of(c, p, "static", "pressure"), s_of(c, p, "air", "temperature"), s_of(c, p, "air", "mach")) else { continue };
                    a.adcs.push(Adc {
                        pitot: pt,
                        stat: ps,
                        tat_air: [t, m],
                        tat_probe: s_of(c, p, "tat_probe", "temperature"),
                        running: s_of(c, p, "computer", "running"),
                        qnh: expr_of(p, "qnh"),
                        vmo: p.num("vmo"),
                        mmo: p.num("mmo"),
                        vs_lag: p.num("vs_lag").max(0.0),
                        last_alt: None,
                        vs: 0.0,
                        s: Slots::of(p, &["ias", "mach", "altitude", "vertical_speed", "tat", "sat", "valid", "failed", "cas", "tas", "pressure_altitude", "overspeed"]),
                    });
                }
                ("computer", "inertial") => {
                    let ids = ["heading", "pitch", "roll", "groundspeed", "on_ground", "latitude", "longitude"];
                    let Some(air) = ids.iter().map(|n| s_of(c, p, "air", n)).collect::<Option<Vec<_>>>() else { continue };
                    a.irs.push(Irs {
                        air: [air[0], air[1], air[2], air[3], air[4], air[5], air[6]],
                        running: s_of(c, p, "computer", "running"),
                        mode: expr_of(p, "mode"),
                        entered: expr_of(p, "position_entered"),
                        align: p.num("align_time").max(1.0),
                        drift: p.num("drift") * 1852.0 / 3600.0,
                        heading_drift: p.num("heading_drift") / 3600.0,
                        restart: p.flag("restart_on_motion"),
                        t: 0.0,
                        nav_t: 0.0,
                        att_t: 0.0,
                        s: Slots::of(p, &["aligning", "aligned", "attitude_valid", "nav_valid", "heading", "pitch", "roll", "position_error", "failed", "align_remaining", "latitude", "longitude"]),
                    });
                }
                ("wear", "counter") => a.wear.push(Wear {
                    rate: expr_of(p, "rate"),
                    service: expr_of(p, "service_when"),
                    cycle: expr_of(p, "cycle_when"),
                    limit: p.num("limit"),
                    serviced: p.num("serviced"),
                    per_cycle: p.num("per_cycle"),
                    life: p.num("life").max(0.0),
                    shape: p.num("shape").max(0.1),
                    last: false,
                    cycles: 0.0,
                    s: Slots::of(p, &["value", "worn", "cycles", "failure_probability"]),
                }),
                _ => {}
            }
        }
        let empty = a.computers.is_empty() && a.links.is_empty() && a.probes.is_empty() && a.tats.is_empty() && a.vanes.is_empty() && a.adcs.is_empty() && a.irs.is_empty() && a.wear.is_empty();
        (!empty).then_some(a)
    }
}

const RHO0: f64 = 1.225;
const P0: f64 = 101_325.0;
const KT: f64 = 0.514_444;

fn ias_from_qc(qc: f64) -> f64 {
    let x = (qc.max(0.0) / P0 + 1.0).powf(0.4 / 1.4) - 1.0;
    (2.0 * 1.4 / 0.4 * P0 / RHO0 * x).sqrt()
}

fn yes(e: &mut Option<Expr>, s: &[f64], dt: f64) -> bool {
    e.as_mut().is_some_and(|x| x.yes(s, dt))
}

impl Domain for Avionics {
    fn name(&self) -> &'static str {
        "avionics"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for c in &mut self.computers {
            let failed = c.s.on(slots, "failed");
            let bus = c.power.is_empty() || c.power.iter().any(|&b| slots[b] != 0.0);
            let powered = bus && c.e[0].as_mut().is_none_or(|e| e.yes(slots, dt)) && !failed;
            let cooled = c.e[1].as_mut().is_none_or(|e| e.yes(slots, dt));
            let bite = yes(&mut c.e[2], slots, dt);
            if yes(&mut c.e[3], slots, dt) {
                c.booted = 0.0;
            }
            c.hot = if powered && !cooled { c.hot + dt } else { (c.hot - dt * 0.5).max(0.0) };
            let overheat = c.overheat_t > 0.0 && c.hot >= c.overheat_t;
            c.unpowered = if powered && !overheat { 0.0 } else { c.unpowered + dt };
            if c.unpowered > c.hold {
                c.booted = 0.0;
            } else if powered && !(bite && c.booted >= c.boot) {
                c.booted += dt;
            }
            let lost = c.s.get(slots, "channel_failed").max(0.0);
            let channel = if lost >= c.channels { 0.0 } else { lost.floor() + 1.0 };
            if channel != c.last_channel && channel > 0.0 && c.last_channel > 0.0 {
                c.transferring = c.transfer;
            }
            c.last_channel = channel;
            c.transferring = (c.transferring - dt).max(0.0);
            let booted = c.booted >= c.boot && !bite;
            let running = c.unpowered <= c.hold && booted && channel > 0.0 && c.transferring <= 0.0;
            let temp = 298.15 + if cooled { 15.0 } else { 60.0 * (c.hot / c.overheat_t.max(60.0)).min(1.5) };
            let s = &c.s;
            s.flag(slots, "powered", powered);
            s.flag(slots, "booting", powered && !booted);
            s.flag(slots, "running", running);
            s.put(slots, "channel", channel);
            s.flag(slots, "degraded", running && (lost >= 1.0 || !cooled));
            s.flag(slots, "transferring", c.transferring > 0.0);
            s.put(slots, "temperature", if powered { temp } else { 298.15 });
            s.flag(slots, "overheat", overheat);
            s.put(slots, "load", if powered && !overheat { if booted { c.load } else { 2.0 * c.load } } else { 0.0 });
        }
        for l in &mut self.links {
            let up = slots[l.from] != 0.0 && !l.s.on(slots, "failed");
            let v = l.value.as_mut().map_or(0.0, |e| e.eval(slots, dt));
            let computed = l.valid_e.as_mut().is_none_or(|e| e.yes(slots, dt));
            let test = yes(&mut l.test_e, slots, dt);
            l.since_send += dt;
            if up && slots[l.frozen] == 0.0 && l.since_send >= l.period {
                l.since_send = 0.0;
                let bad = slots[l.bad] != 0.0;
                let sent = if bad { v * 1.37 + 11.0 } else { v };
                // ARINC 429 BNR sign/status matrix: 1 no computed data, 2 functional test, 3 normal. Bad data is the
                // undetected kind: wrong values under a normal status, for the receivers' own monitoring to catch.
                let ssm = if test { 2.0 } else if computed || bad { 3.0 } else { 1.0 };
                let q = if l.range > 0.0 {
                    let lsb = l.range / (1u64 << l.bits) as f64;
                    (sent.clamp(-l.range, l.range - lsb) / lsb).round() * lsb
                } else {
                    sent
                };
                l.queue.push((l.latency, q, ssm));
            }
            for q in &mut l.queue {
                q.0 -= dt;
            }
            while l.queue.first().is_some_and(|q| q.0 <= 0.0) {
                let (_, v, ssm) = l.queue.remove(0);
                l.held = v;
                l.ssm = ssm;
                l.age = 0.0;
            }
            l.age += dt;
            let valid = up && l.age <= l.stale && l.ssm == 3.0;
            let status = if valid { 0.0 } else if up { 1.0 } else { 2.0 };
            l.s.put(slots, "value", l.held);
            l.s.put(slots, "status", status);
            l.s.put(slots, "age", l.age.min(1e6));
            l.s.flag(slots, "valid", valid);
            l.s.put(slots, "ssm", l.ssm);
            l.s.put(slots, "word", word(l.label, l.sdi, l.ssm as u32, l.held, l.range, l.bits));
        }
        for pr in &mut self.probes {
            let (p, v, icing, t) = (slots[pr.air[0]], slots[pr.air[1]], slots[pr.air[2]], slots[pr.air[3]].max(150.0));
            let failed = pr.s.on(slots, "failed");
            let blocked_e = pr.blocked.as_mut().is_some_and(|e| e.yes(slots, dt));
            let m = v / (1.4 * 287.05 * t).sqrt();
            if pr.pitot {
                let heated = pr.heat.as_mut().is_some_and(|e| e.yes(slots, dt)) && !failed;
                // Probe skin: kinetic heating plus its heater against convection, which grows with speed.
                let t_rec = t * (1.0 + 0.2 * 0.9 * m * m);
                let conductance = 1.0 + 0.25 * v.max(0.0).sqrt();
                let goal = t_rec + if heated { pr.heater / conductance } else { 0.0 };
                pr.temp += (goal - pr.temp) * (1.0 - (-dt / 20.0).exp());
                let freezing = pr.temp < 273.15 && icing > 0.0;
                pr.blockage = if freezing { (pr.blockage + icing * dt / pr.block_time).min(1.0) } else { (pr.blockage - dt / 10.0).max(0.0) };
            }
            let blocked = (pr.pitot && pr.blockage >= 1.0) || failed || blocked_e;
            if !blocked {
                let reading = if pr.pitot {
                    p * (1.0 + 0.2 * m * m).powf(3.5)
                } else {
                    let q = 0.5 * p / (287.05 * t) * v * v;
                    let beta = pr.beta.map_or(0.0, |b| slots[b].to_radians().abs());
                    p + q * (pr.error + pr.beta_error * beta)
                };
                pr.held = if pr.lag > 0.0 { pr.held + (reading - pr.held) * (1.0 - (-dt / pr.lag).exp()) } else { reading };
            }
            pr.s.put(slots, "pressure", pr.held);
            if pr.pitot {
                pr.s.flag(slots, "iced", pr.blockage >= 1.0);
                pr.s.put(slots, "temperature", pr.temp);
                pr.s.put(slots, "blockage", pr.blockage);
            }
        }
        for tp in &mut self.tats {
            let (t, m, v) = (slots[tp.air[0]].max(150.0), slots[tp.air[1]], slots[tp.air[2]].abs());
            let heated = yes(&mut tp.heat, slots, dt);
            // Heater error: aspiration-free probes read high on the ground, fading by about 100 kt.
            let heat_err = if heated { 25.0 * (1.0 - v / (100.0 * KT)).clamp(0.0, 1.0) } else { 0.0 };
            let goal = t * (1.0 + 0.2 * tp.recovery * m * m) + heat_err;
            if !tp.s.on(slots, "failed") {
                tp.t += (goal - tp.t) * (1.0 - (-dt / tp.lag).exp());
            }
            tp.s.put(slots, "temperature", tp.t);
        }
        for vn in &mut self.vanes {
            let (aoa, icing, t) = (slots[vn.air[0]], slots[vn.air[1]], slots[vn.air[2]]);
            let heated = yes(&mut vn.heat, slots, dt);
            vn.ice = if !heated && t < 273.15 && icing > 0.0 { (vn.ice + icing * dt / vn.freeze).min(1.0) } else { (vn.ice - dt / 20.0).max(0.0) };
            let frozen = vn.ice >= 1.0 || vn.s.on(slots, "failed");
            if !frozen {
                vn.angle += (aoa - vn.angle) * (1.0 - (-dt / vn.lag).exp());
            }
            vn.s.put(slots, "angle", vn.angle);
            vn.s.flag(slots, "frozen", vn.ice >= 1.0);
        }
        for a in &mut self.adcs {
            let ok = a.running.is_none_or(|r| slots[r] != 0.0) && !a.s.on(slots, "failed");
            let (pt, ps) = (slots[a.pitot], slots[a.stat].max(1.0));
            let qnh = a.qnh.as_mut().map_or(P0, |e| e.eval(slots, dt)).max(1.0);
            let mach = ((2.0 / 0.4) * ((pt.max(ps) / ps).powf(0.4 / 1.4) - 1.0)).max(0.0).sqrt();
            let alt = 44_330.8 * (1.0 - (ps / qnh).powf(0.190_263));
            let std_alt = 44_330.8 * (1.0 - (ps / P0).powf(0.190_263));
            let raw_vs = a.last_alt.map_or(0.0, |l| (alt - l) / dt.max(1e-6));
            a.last_alt = Some(alt);
            a.vs = if a.vs_lag > 0.0 { a.vs + (raw_vs - a.vs) * (1.0 - (-dt / a.vs_lag).exp()) } else { raw_vs };
            let tat = a.tat_probe.map_or_else(|| slots[a.tat_air[0]] * (1.0 + 0.2 * slots[a.tat_air[1]].powi(2)), |p| slots[p]);
            let sat = tat / (1.0 + 0.2 * mach * mach);
            let cas = ias_from_qc(pt - ps);
            let tas = mach * (1.4 * 287.05 * sat.max(150.0)).sqrt();
            let overspeed = (a.vmo > 0.0 && cas > a.vmo + 2.0 * KT) || (a.mmo > 0.0 && mach > a.mmo + 0.005);
            let vals = [cas, mach, alt, a.vs, tat, sat, cas, tas, std_alt];
            for (name, v) in ["ias", "mach", "altitude", "vertical_speed", "tat", "sat", "cas", "tas", "pressure_altitude"].iter().zip(vals) {
                a.s.put(slots, name, if ok { v } else { 0.0 });
            }
            a.s.flag(slots, "valid", ok);
            a.s.flag(slots, "overspeed", ok && overspeed);
        }
        for i in &mut self.irs {
            let ok = i.running.is_none_or(|r| slots[r] != 0.0) && !i.s.on(slots, "failed");
            let mode = i.mode.as_mut().map_or(1.0, |e| e.eval(slots, dt)).round();
            let still = slots[i.air[4]] != 0.0 && slots[i.air[3]].abs() < 0.5;
            let entered = i.entered.as_mut().is_none_or(|e| e.yes(slots, dt));
            let lat = slots[i.air[5]];
            // Gyrocompassing slows toward the poles: the published ADIRU figure, 10 minutes to 60 degrees, 17 by 78.
            let needed = i.align * (1.0 + 0.7 * ((lat.abs() - 60.0) / 18.0).clamp(0.0, 1.0));
            if !ok || mode == 0.0 {
                i.t = 0.0;
                i.nav_t = 0.0;
                i.att_t = 0.0;
            } else if mode == 2.0 {
                i.att_t += dt;
            } else if i.t < needed || !entered {
                if still {
                    i.t = (i.t + dt).min(needed);
                } else if i.restart && i.t > 0.0 && i.t < needed {
                    i.t = 0.0;
                }
            } else {
                i.nav_t += dt;
            }
            let aligned = ok && mode == 1.0 && i.t >= needed && entered;
            let att = ok && (aligned || (mode == 2.0 && i.att_t >= 30.0));
            let s = &i.s;
            s.flag(slots, "aligning", ok && mode == 1.0 && !aligned);
            s.flag(slots, "aligned", aligned);
            s.flag(slots, "attitude_valid", att);
            s.flag(slots, "nav_valid", aligned);
            let hdg_err = if mode == 2.0 { i.heading_drift * i.att_t } else { 0.0 };
            for (k, name) in ["heading", "pitch", "roll"].iter().enumerate() {
                let v = slots[i.air[k]] + if k == 0 { hdg_err } else { 0.0 };
                s.put(slots, name, if att { v } else { 0.0 });
            }
            // Drift grows with time, with the 84.4 minute Schuler oscillation riding on it.
            let schuler = (std::f64::consts::TAU * i.nav_t / 5064.0).sin().abs() * i.drift * 600.0;
            let err = if aligned { i.drift * i.nav_t + schuler } else { 0.0 };
            s.put(slots, "position_error", err);
            s.put(slots, "align_remaining", if ok && mode == 1.0 { (needed - i.t).max(0.0) } else { 0.0 });
            let north = err / 111_120.0 * 0.7;
            s.put(slots, "latitude", if aligned { lat + north } else { 0.0 });
            s.put(slots, "longitude", if aligned { slots[i.air[6]] + north } else { 0.0 });
        }
        for w in &mut self.wear {
            let mut v = w.s.get(slots, "value") + w.rate.as_mut().map_or(0.0, |e| e.eval(slots, dt)) * dt;
            let c = yes(&mut w.cycle, slots, dt);
            if c && !w.last {
                w.cycles += 1.0;
                v += w.per_cycle;
            }
            w.last = c;
            if yes(&mut w.service, slots, dt) {
                v = w.serviced;
            }
            let pf = if w.life > 0.0 { 1.0 - (-(v.max(0.0) / w.life).powf(w.shape)).exp() } else { 0.0 };
            w.s.put(slots, "value", v);
            w.s.flag(slots, "worn", v >= w.limit);
            w.s.put(slots, "cycles", w.cycles);
            w.s.put(slots, "failure_probability", pf);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        for c in &self.computers {
            for v in [c.booted, c.unpowered.min(1e9), c.last_channel, c.transferring, c.hot] {
                p.put(v);
            }
        }
        for l in &self.links {
            for v in [l.held, l.age.min(1e9), l.since_send, l.ssm] {
                p.put(v);
            }
        }
        for x in &self.probes {
            for v in [x.held, x.temp, x.blockage] {
                p.put(v);
            }
        }
        self.tats.iter().for_each(|x| p.put(x.t));
        for x in &self.vanes {
            p.put(x.angle);
            p.put(x.ice);
        }
        for a in &self.adcs {
            p.put(a.vs);
        }
        for i in &self.irs {
            for v in [i.t, i.nav_t, i.att_t] {
                p.put(v);
            }
        }
        for w in &self.wear {
            p.put(w.cycles);
            p.put(w.last as u8 as f64);
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        for c in &mut self.computers {
            c.booted = p.take();
            c.unpowered = p.take();
            c.last_channel = p.take();
            c.transferring = p.take();
            c.hot = p.take();
        }
        for l in &mut self.links {
            l.held = p.take();
            l.age = p.take();
            l.since_send = p.take();
            l.ssm = p.take();
            l.queue.clear();
        }
        for x in &mut self.probes {
            x.held = p.take();
            x.temp = p.take();
            x.blockage = p.take();
        }
        self.tats.iter_mut().for_each(|x| x.t = p.take());
        for x in &mut self.vanes {
            x.angle = p.take();
            x.ice = p.take();
        }
        for a in &mut self.adcs {
            a.vs = p.take();
            a.last_alt = None;
        }
        for i in &mut self.irs {
            i.t = p.take();
            i.nav_t = p.take();
            i.att_t = p.take();
        }
        for w in &mut self.wear {
            w.cycles = p.take();
            w.last = p.take() != 0.0;
        }
    }
}
