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

Licence: MPL-2.0.
