# EasyAcft

A whole-aircraft systems library for X-Plane. You describe the aircraft in TOML files, the checker catches mistakes before anything runs, and a plugin runs the systems in the sim.

Covered: electrical, hydraulic, pneumatic and bleed, fuel, gas turbines and pistons with propellers, FADEC, APU, gear and brakes, pressurisation, ice, oxygen, fire protection, lights, sounds, failures and breakers, computers and data links, air data and inertial sensors, wear, weight and balance.

Not covered on purpose: flight-management, autopilot and flight-control laws. Write your own and run them as EasyAcft computers through the C API.

## Start

```
easyacft check examples/twin-jet       # check an aircraft
easyacft test examples/twin-jet        # run its scenario tests
easyacft-studio examples/twin-jet      # see and edit every system live
```

`easyacft kinds` lists every part. `easyacft kinds electrical.generator` shows its settings and values.

## An aircraft

```toml
[aircraft]
name = "My Twin"
easyacft = "1.0"
include = ["systems/electrical.toml"]

[bindings]
"sim/cockpit2/electrical/battery_on[0]" = { to = "bat_sw.on" }
"sim/cockpit2/electrical/bus_volts[0]" = "dc1.voltage"
```

```toml
[[electrical.battery]]
id = "bat"
bus = "dc1"
voltage = "24 V"
capacity = "44 Ah"

[[electrical.bus]]
id = "dc1"
nominal = "28 V"

[[electrical.contactor]]
id = "glc{n}"
count = 2
between = ["gen{n}_out", "ac{n}"]
close_when = "gen{n}_sw.on && gen{n}.ready && delay(gen{n}.ready, 1 s)"
```

Values carry units and the checker holds every setting and expression to the right one. Expressions read any part's values and have `delay`, `off_delay`, `latch`, `rising`, `min`, `max`, `abs`, `if` and `table('file.csv', x)`.

A variant changes another aircraft:

```toml
[variant]
of = "../twin-jet"
remove = ["galley"]

[[engine.gas_turbine]]
id = "eng{n}"
count = 2
design_thrust = "130 kN"
```

## Your own code

Most settings that drive a network can be expressions, so your code steers the built-in parts: a load's `power`, a pump's or consumer's `flow`. For a part kind EasyAcft lacks, declare it and step it yourself:

```toml
[kinds.turbo_pump]
help = "A turbine-driven pump"
settings.rated_flow = "L/min"
settings.drive = { type = "expr bool", default = "false" }
settings.circuit = "part hydraulic.circuit"
values.flow = "L/min"

[[custom.turbo_pump]]
id = "tp1"
rated_flow = "40 L/min"
drive = "eng1.running"
circuit = "green"

[[hydraulic.pump]]
id = "tp1_out"
circuit = "green"
pressure = "3000 psi"
flow = "tp1.flow"
```

```c
void turbo_pump(easyacft_part *p, double dt, void *user) {
    double on = easyacft_part_get(p, "drive");
    easyacft_part_set(p, "flow", on ? easyacft_part_setting(p, "rated_flow") : 0.0);
}
easyacft_on_kind(aircraft, "turbo_pump", turbo_pump, NULL);
```

A custom part can also sit in a network itself. A port joins one bus, circuit, duct or line and behaves like a source of level `e` behind conductance `g`, so flow = g × (e − v); a link is a conductance between two of them:

```toml
[kinds.vfg]
settings.droop = "ohm"
ports.out = "electrical source"

[kinds.ssr]
settings.closed = "expr bool"
links.poles = "electrical"
```

```c
void vfg(easyacft_part *p, double dt, void *user) {
    double v = easyacft_part_get(p, "out_v");                 /* bus voltage after the last solve */
    easyacft_part_set(p, "out_e", 115.0 + 0.5 * (115.0 - v)); /* your regulator */
    easyacft_part_set(p, "out_g", 1.0 / easyacft_part_setting(p, "droop"));
}
void ssr(easyacft_part *p, double dt, void *user) {
    easyacft_part_set(p, "poles_g", easyacft_part_get(p, "closed") ? 1000.0 : 0.0);
}
```

Ports take `electrical`, `hydraulic`, `pneumatic` or `fuel`, optionally followed by `source` or `load` for how Studio draws them. The network writes `<port>_v` and `<port>_q` (and `<link>_q`) back after every solve. Load shedding does not count a custom source's capacity.

The checker holds custom parts to their declaration like any other, and Studio shows them. From your own X-Plane plugin, ask the EasyAcft plugin for the same functions with `EASYACFT_MSG_GET_API` (see `easyacft.h`); your handlers survive hot reloads. Rust code calls `Aircraft::on_kind`. Any plugin, in any language, can also read and write every value through its dataref.

## Export to code

```
easyacft export examples/twin-jet my-twin
cd my-twin && cargo test
```

writes the aircraft as a Rust crate that needs nothing from EasyAcft: `src/lib.rs` holds every part and setting, every expression as a plain Rust function and a constant for every value (`v::DC1_VOLTAGE`); `runtime/` holds the solver and part models; the scenario tests become Rust tests. It has no outside dependencies, and LICENSE-EXPORT.md lets you use, change and ship it under any terms. Studio has the same under Export to code.

## Tests

```toml
[[step]]
at = "0 s"
set = { "bat_sw.on" = 1 }

[[expect]]
at = "2 s"
that = "dc1.voltage > 22 V"
```

Put these in `<aircraft>/tests/`. Studio records them for you: press Record test, work the aircraft, press Stop.

## Studio

- every system drawn from the files, live: heavy line supplied, thin line off, dashed failed;
- click a part for its values, why it is in that state, and its settings; save writes the TOML and keeps your comments;
- shift-drag between two buses or lines to connect them, right-click to fail;
- engine side view with the design point solving as you type, engine fit against test points;
- plots, maintenance log, health, tables, computers, variant compare;
- try, keep or discard; replay the last five minutes; undo;
- attach to a running X-Plane.

`easyacft-studio <dir> --shot out.png` saves a screenshot; `--svg <dir> <domain> out.svg` exports a diagram.

## X-Plane

Build `easyacft-plugin`, rename it `easyacft.xpl`, put it in the aircraft's `plugins` folder and the TOML files in `<aircraft>/easyacft/`. Every value is published as `easyacft/<id>/<value>`, and every switch gets `easyacft/<id>/toggle`, `/on` and `/off` commands. Plugins > EasyAcft > Systems window shows every part with fail and repair.

## Import

```
easyacft import MyPlane.acf my-plane
easyacft import "MSFS aircraft folder" my-plane
```

Starts an aircraft from an X-Plane `.acf` or an MSFS aircraft, ready to check and edit.

## Examples

- `examples/twin-jet`: a twin jet with every system, the proof aircraft;
- `examples/piston-single`: a four-seat single;
- `examples/a380`: A380-800 systems from public data; not affiliated with or endorsed by Airbus.

## C API

`crates/easyacft-ffi/include/easyacft.h`: load, step, find, get, set, snapshots, computers.

Licence: MPL-2.0, with the export exception in LICENSE-EXPORT.md for code `easyacft export` writes.
