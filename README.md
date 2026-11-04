# EasyAcft

A whole-aircraft systems library for X-Plane. You describe the aircraft in TOML files, the checker catches mistakes before anything runs, and a plugin runs the systems in the sim.

Covered: electrical, hydraulic, pneumatic and bleed, fuel, gas turbines and pistons with propellers, FADEC, APU, gear and brakes, pressurisation, ice, oxygen, fire protection, lights, sounds, failures and breakers, computers and data links, air data and inertial sensors, wear, weight and balance.

Not covered on purpose: flight-management, autopilot and flight-control laws. Write your own and run them as EasyAcft computers through the C API.

## Build from source

You need Rust 1.82 or newer ([rustup.rs](https://rustup.rs)) and a C linker:

- Windows: the Visual Studio Build Tools ("Desktop development with C++"), or the GNU toolchain (`rustup toolchain install stable-x86_64-pc-windows-gnu`);
- macOS: the Xcode command line tools (`xcode-select --install`);
- Linux: `gcc` (`sudo apt install build-essential` on Debian and Ubuntu).

No X-Plane SDK is needed: the plugin finds X-Plane's functions when X-Plane loads it.

```
git clone https://github.com/Vihaan2012-cmyk/EasyAircraft.git
cd EasyAircraft
cargo build --release
cargo test --workspace --release
```

Everything lands in `target/release/`:

| What | Windows | macOS | Linux |
|---|---|---|---|
| command line | `easyacft.exe` | `easyacft` | `easyacft` |
| Studio | `easyacft-studio.exe` | `easyacft-studio` | `easyacft-studio` |
| X-Plane plugin | `easyacft_plugin.dll` | `libeasyacft_plugin.dylib` | `libeasyacft_plugin.so` |
| C library | `easyacft.dll`, `easyacft.lib` | `libeasyacft.dylib`, `libeasyacft.a` | `libeasyacft.so`, `libeasyacft.a` |

Build one part with `-p`, for example `cargo build --release -p easyacft-studio`. To run `easyacft` and `easyacft-studio` from anywhere, `cargo install --path crates/easyacft-cli` and `cargo install --path crates/easyacft-studio`. Studio on Linux needs OpenGL and an X11 or Wayland desktop.

To install the plugin, rename it `easyacft.xpl` and put it in `<aircraft>/plugins/easyacft/win_x64/`, `mac_x64/` or `lin_x64/` for your system. For one Mac plugin that runs on both Apple and Intel chips:

```
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release -p easyacft-plugin --target aarch64-apple-darwin
cargo build --release -p easyacft-plugin --target x86_64-apple-darwin
lipo -create -output easyacft.xpl target/aarch64-apple-darwin/release/libeasyacft_plugin.dylib target/x86_64-apple-darwin/release/libeasyacft_plugin.dylib
```

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

## How it runs

- **Rates.** Each domain steps at its own period: logic, electrical and engines 10 ms, hydraulic 5 ms, pneumatic, controls and the ready-made systems 20 ms, fuel 50 ms. Change them under `[rates]`, for example `electrical = "5 ms"`.
- **One step behind.** A domain reads every value as the last step left it. A chain of parts that each wait on the one before settles one link per step: five contactors in a row take 50 ms at 10 ms.
- **Networks.** Each step is one linear solve per island of connected nodes, with no iteration. An electrical island whose sources and loads have not changed is not solved again; hydraulic, pneumatic and fuel islands hold pressure between steps, so they solve every step.
- **Loads** draw constant power, linearised at the last step's bus voltage. Below `min_voltage` (default half the bus nominal) a load switches itself off and draws nothing; it comes back 0.5 s after its bus is 10 % above that. A constant-power load cannot be solved below about half voltage in one linear step without oscillating, so the dropout comes first.
- **Batteries.** Open-circuit voltage runs from 0.88 to 1.05 of nominal over 10 to 100 % charge and falls to zero below 10 %, so a flat battery stops holding its bus up.
- **Generators.** `rating` is apparent power (kVA); `power_factor`, default 1, sets the real power load shedding counts on.
- **Expressions.** `&&`, `||` and `if` evaluate only the side they take. A `delay`, `off_delay`, `latch` or `rising` on a side not taken resets, so `if(armed, latch(fault, reset), false)` only latches while armed. `off_delay` starts expired. Division by zero gives 0.
- **Units.** Torque and energy share a dimension (N*m is J), so the checker accepts `"500 J"` where a torque goes; rpm and Hz are both frequency (60 rpm is 1 Hz).
- **Cost.** `easyacft profile <aircraft dir> [--scenario f] [--budget 1ms]` times every domain. The A380 example (2,035 parts, engines running) averages about 0.2 ms per 60 Hz frame.

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

Build `easyacft-plugin` and install it as in [Build from source](#build-from-source), and put the TOML files in `<aircraft>/easyacft/`. Every value is published as `easyacft/<id>/<value>`, and every switch gets `easyacft/<id>/toggle`, `/on` and `/off` commands. Plugins > EasyAcft > Systems window shows every part with fail and repair.

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
