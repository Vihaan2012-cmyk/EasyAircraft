# EasyAcft plan

Draft for review, 3 Nov 2026. Status at the end.

EasyAcft is a whole-aircraft systems library for X-Plane, for any aircraft developer. It is a separate
product from the A380 and shares no code with it.

**Goal:** cut the systems workload of building an X-Plane aircraft as far as possible. Anything most
aircraft need, EasyAcft ships ready-made and configurable, so developers write only what is unique to
their aircraft.

## Already decided

- X-Plane only (12 first; 11 if cheap).
- Rust core, C header (`easyacft.h`), so C/C++ plugins and Rust plugins both use it.
- An aircraft is described in TOML files. Anything the files can't express goes through the C API.
- v1 covers the whole aircraft: electrical, hydraulic, pneumatic/bleed, fuel, engines, APU, lights,
  failures, breakers, generic logic, and the power/health/data side of every computer.
- EasyAcft ships a complete configurable FADEC.
- Not in EasyAcft: LNAV/VNAV, FMS, autopilot, autothrust or flight-control laws. Developers write
  their own and run them inside EasyAcft computers.
- The engine is a generic gas path, and the developer can change every part of it.
- A rustc-style checker (`easyacft check`) for the aircraft files.
- MPL-2.0. Aircraft that use EasyAcft can stay closed. Only changes to EasyAcft's own source files must
  be published.
- Clean room: written fresh, from textbooks and public data.

## Decisions I need from you

1. **How it ships.** Recommended: **A**.
   - **A:** a library each aircraft links into its own plugin. Every aircraft pins its own version,
     so there are no clashes between aircraft.
   - **B:** one shared EasyAcft plugin that every aircraft loads. Updates are smaller, but two aircraft
     can need different versions.
2. **Logic in files.** Recommended: **A**.
   - **A:** v1 has a small expression language in the TOML (conditions, timers, latches) plus C
     callbacks for anything bigger.
   - **B:** also embed Lua in v1, like SASL/FlyWithLua users expect. That is more work and gives
     the checker less to check.
3. **Engines in v1: decided, generic gas path, every part of it editable** (see "Engines" below).
   Table-driven engines move to after v1.
4. **Proof aircraft.** Recommended: both.
   - A clean-room generic twin-jet and a single piston, each built only from EasyAcft files, as
     examples and as the test suite's real aircraft.
5. **Name: decided, EasyAcft.**

## What makes it versatile

The core idea is that a few general parts compose into any aircraft, instead of one hand-written
model per aircraft type.

1. **One network solver for every flow.** Electrical (volts/amps), hydraulic (pressure/flow),
   pneumatic (pressure/temperature/mass flow) and fuel (mass/level with gravity feed) are all nodes,
   links and sources on the same solver.
   - Each domain supplies only its own laws.
   - A C172's battery and bus and an A380's 26 buses are the same machinery at different sizes.
2. **Components with ports.** The library is a catalogue of parts with typed ports and parameters:
   battery, generator, TRU, inverter, contactor, breaker, pump, accumulator, valve, duct, tank,
   heat exchanger and so on.
   - An aircraft is instances plus wires.
   - A port's type stops you wiring a hydraulic line to a DC bus, and the checker reports it.
3. **Expressions in the files.** Contactor logic, priorities, timers and latches are written in a
   small typed expression language, for example `close_when = "gen1.ready && !bus_tie.auto_off"`.
   - The checker compiles the expressions to bytecode.
   - Typos, unit mismatches and unknown names fail at check time, not in the sim.
4. **Units everywhere.** Values carry units: `28 V`, `3000 psi`, `45 kVA`, `120 s`.
   - The checker converts them and rejects `voltage = 3000 psi`.
5. **Templates, instances and variants.**
   - Define a part once and instance it N times: `[[engine]] template = "turbofan" count = 4`.
   - Files can include other files.
   - Variant files override parameters, for engine options, weight variants and customer options.
6. **Failures and breakers for free.**
   - Every component gets failure modes from its type automatically (open, short, leak, seized,
     degraded), with stable failure ids.
   - Every feed can carry a breaker.
   - The developer only adds failures that are specific to their aircraft.
7. **Choose how much of X-Plane to replace.** Each system can do one of three things:
   - drive X-Plane's own datarefs, so default X-Plane systems follow it;
   - publish custom datarefs and commands only;
   - leave X-Plane's default system running.

   This lets a developer adopt EasyAcft one system at a time.
8. **Custom parts through the C API.** A developer registers a component type in C: ports,
   parameters, and a step function. Their part then behaves like a built-in one, checker included.
9. **Deterministic stepping.**
   - Fixed sub-steps per domain, tied to sim time, never wall-clock time.
   - Snapshots save and restore the whole state, which gives save/load, replay and test fixtures.
10. **Headless runs.** The same files run outside X-Plane (`easyacft sim`) with scripted scenarios, so
    systems can be tested in CI without the sim.

## How much control developers have

Each level is optional. A developer goes only as far down as their aircraft needs.

1. **Parameters.** Every number of every built-in part is in the files, including:
   - the segment count of any pipe, duct or cable (`segments = 12`);
   - each domain's step rate (`[rates] hydraulic = "2 ms"`, `thermal = "100 ms"`);
   - each force sent to X-Plane: which engine or part, its position on the airframe, its
     direction, and a scale.
2. **Expressions.** Logic is written in the files: conditions, timers, latches, schedules and
   tables. They can also:
   - change a step rate while the sim runs, for example faster during an engine start;
   - gate or blend a force, for example reverser thrust only with the doors open, or RAT drag
     rising as it deploys.
3. **Replace a built-in controller.** The FADEC, APU controller, pressurization controller and so
   on can each be swapped for a C callback, while the physics underneath stays EasyAcft's. The same
   goes for force output: a developer can take each computed force and apply it with their own
   code, for example to add their own nacelle drag or spread thrust across several points.
4. **Custom parts.**
   - The developer registers a new part type in C with typed ports and its own step function.
   - It joins the solver like a built-in part. For example, a fuel cell is an electrical source
     with the developer's own voltage-current curve.
5. **Custom domains.**
   - The developer defines a new kind of network in the files: what flows and what drives it,
     for example coolant mass flow driven by pressure, carrying heat.
   - It reuses the same solver, failures and checker. Oil, glycol cooling loops, vacuum
     instruments, nitrogen inerting, water ballast and lifting gas all fit here.
6. **Hooks on anything.**
   - Read or write any EasyAcft value every frame, before or after any part steps.
   - Force a value, for training scenarios or for the developer's own code to drive.
7. **Take over at a boundary.**
   - Switch off any built-in system, or part of one.
   - An external port lets the developer's own code supply that piece. For example, EasyAcft runs
     the electrical system and the developer's existing hydraulic code is a load on it.
8. **Change EasyAcft itself.** MPL-2.0 allows it; only the changed EasyAcft files must be published.

**Limits:** the solver handles lumped networks (nodes and links) at a fixed time step. Two
additions push that limit out cheaply:

- **Segmented links.** A long pipe, duct or cable can be split into N segments. That gives
  pressure waves, transport delay, and heat moving along a bleed duct or fuel line.
- **Per-domain rates.** Each domain steps at its own rate: electrical and hydraulic fast,
  thermal slow.

EasyAcft also hands forces back to X-Plane: engine, propeller and reverser thrust, RAT drag, and
moments from asymmetric thrust. A gas-path engine flies the aircraft, not just the gauges.

Physics that needs a field solution (airflow over surfaces, structural stress, combustion
chemistry) stays in the developer's code or in X-Plane's flight model, connected through levels
4, 6 or 7.

## File format sketch

```toml
[aircraft]
name = "Example Twin"
easyacft = "1.0"

[[electrical.battery]]
id = "bat1"
capacity = "44 Ah"
voltage = "24 V"

[[electrical.bus]]
id = "dc1"
nominal = "28 V"

[[electrical.contactor]]
id = "blc1"
between = ["bat1", "dc1"]
close_when = "switch.bat1 && !dc1.powered_by(gen1)"

[[engine]]
template = "turbofan"
count = 2
params = { rated_thrust = "120 kN", bypass_ratio = 5.0 }

[bindings]
"sim/cockpit2/electrical/bus_volts[0]" = "dc1.voltage"
"easyacft/elec/bat1/amps" = { from = "bat1.current", unit = "A" }
```

## Checker

`easyacft check aircraft/` reads every file, resolves names and units, compiles the expressions and
prints rustc-style diagnostics with codes, spans and fixes:

```
error[E0207]: port `hyd.green.out` (hydraulic) cannot connect to `dc1` (electrical)
  --> systems/hydraulic.toml:14:12
   |
14 | to = "dc1"
   |      ^^^^^ electrical bus
   = help: a hydraulic port connects to a hydraulic line, accumulator or consumer
```

The checker also has these subcommands:
- `easyacft graph`: exports the network as SVG.
- `easyacft explain E0207`: explains an error code.
- `easyacft fmt`: formats the files.

An editor plugin (an LSP) comes after v1.

## Engines

The gas path is built in the files from stations and stages, so the developer defines the engine
instead of picking from a fixed list.

- **Layout:**
  - Any order of the stage types: inlet, fan with core/bypass split, booster, compressors,
    burner, turbines, ducts, mixer and nozzles.
  - 1 to 3 spools.
  - Optional fan gearbox, free power turbine with a propeller gearbox, and a mixed or separate
    exhaust.
  - One file covers a turbojet, a high-bypass turbofan, a geared fan or a turboprop.
- **Every stage's numbers:**
  - stage count, design pressure ratio, efficiency, flow capacity and radius;
  - either a generic parametric map or the developer's own map tables;
  - variable stator schedules, stall line, and degradation inputs.
- **Air taken off anywhere:**
  - Customer bleed ports with switch-over logic.
  - Handling bleeds with schedules.
  - Turbine cooling flows from any port to any station.
  - Anti-ice and seal leakage.
  - Each entry sets its own source station, destination and amount.
- **Burner and gas:** combustion efficiency, pressure loss, fuel heating value, stoichiometric
  limit, and the heat-capacity model.
- **Spools:** inertia, friction, bearing loads, and gearbox offtakes for generators and pumps,
  which the other domains load.
- **Control, also in the files:**
  - the governed variable (N1, EPR, torque or N2);
  - acceleration and deceleration schedules;
  - TGT, speed and torque limiters;
  - start and relight schedules, idle modes, rating tables and derates.
  - A developer can replace the whole controller with a C callback.
- **Design point:** the developer gives targets (thrust, bypass ratio, pressure ratio, airflow), and
  EasyAcft solves the turbine-entry temperature and the areas.
- **Calibration tool:** `easyacft engine fit` tunes chosen parameters against test points the
  developer supplies, for example ICAO fuel flows, take-off EGT and an acceleration time. It
  reports the error at each point.
- **Custom stages:** a new stage type can be registered through the C API and placed in the
  layout like a built-in one.

### FADEC

EasyAcft ships a complete, configurable FADEC. It runs on the EasyAcft computer parts (see "Computers"),
so it has real power, channels and failures.

- **Thrust lever to thrust:**
  - Lever detents and ranges come from the files: TOGA, FLX/MCT, CL, IDLE, reverse, or any set.
  - Rating tables by altitude, temperature, Mach and bleed state: take-off, max continuous, climb,
    flexible and derated take-off, derated climb.
- **Governing:**
  - The governed variable is N1, EPR, torque or N2.
  - Acceleration and deceleration schedules.
  - Idle modes: minimum, approach and ground idle, plus idle raised for anti-ice or bleed demand.
- **Protection:** overspeed, TGT and torque limiters; surge detection and recovery; flame-out
  detection with auto-relight.
- **Start and relight:**
  - Automatic and manual start sequences with valve, ignition and fuel timing.
  - Auto-abort on hot, hung or no-light starts, with retries and dry cranks.
  - Windmill and starter-assisted relight envelopes.
- **Interlocks:** reverser deploy and stow logic, take-off-config inputs, and thrust-lever
  disagreement handling.
- **Channels:**
  - EEC channel A/B with active/standby swap on fault.
  - Degraded modes when sensors fail, such as EPR to N1 reversion and fixed fuel schedules.
  - Fault messages for the developer's warning system.
- **Inputs from the developer's code:**
  - an autothrust target, and an alpha-floor or TOGA-lock request;
  - any extra limiter they want, through a C callback.

  The developer's autothrust law decides when; the FADEC decides how.
- **Editable throughout:** every schedule, table, threshold and timer is in the files, and the
  checker validates them (for example, tables must be monotonic and cover the flight envelope).

Piston engines are a separate model: cylinders, manifold, mixture, magnetos, turbo/supercharger and
propeller. They are just as editable.

## Ready-made systems

Each one is a few parts plus a small controller on top of the solver, expressions, failures and
computers, so each is cheap to add. All are configured in the files, and each can be replaced at
levels 3-7.

| System | What it gives the developer |
|---|---|
| Air data and inertial sensors | Pitot, static, AOA and temperature probes; air data computer (IAS, Mach, altitude, TAT/SAT); blockage and icing failures; IRS alignment time, drift and modes |
| Cockpit switch behaviour | Guarded switches, latching push-buttons with FAULT/OFF/ON lights, rotary selectors, annunciator test, pull/push breakers bound to 3D cockpit clicks |
| Pressurization | Cabin volume, outflow valves, leakage, safety valves, cabin altitude schedule, auto and manual controller, differential pressure limits |
| Fire protection | Detection loops A/B with fault and fire states, bottles, squibs, discharge, agent concentration |
| APU controller | Start sequence, speed governing, bleed and generator load, protective shutdowns |
| Wear and servicing | Oil and hydraulic fluid use, brake and tyre wear, battery state, filter clogging; kept between flights |
| Landing gear and brakes | Gear and door sequencing, gravity extension, brake accumulator, brake temperatures, hydraulic steering |
| Ice and rain protection | Wing, engine, probe and window heat; ice build-up that feeds the air data and engine models |
| Fuel management | Transfer and crossfeed logic, quantity indication with probe errors, CG transfer |
| Electrical load shedding | Priority tables, shedding on generator loss or overload |
| Oxygen | Crew bottle pressure and use, passenger masks at a set cabin altitude, chemical generator time |
| Weight and balance | Fuel and payload to mass and CG, written to X-Plane |
| Maintenance log | Every failure and computer fault with time and flight phase, ready for maintenance pages |
| Sound events | Contactor clunks, pump and fan sounds following their speeds, published as datarefs for FMOD |
| Turboprop extras | Propeller governor, feathering, beta range, starter-generators |

## Computers (EasyAcft hosts them, the developer writes them)

EasyAcft does not write flight logic. LNAV/VNAV, FMS, autopilot, autothrust, flight-control laws and
warning logic belong to the developer; the FADEC is the one computer EasyAcft ships itself. EasyAcft
gives every computer the parts real aircraft computers depend on:

- **Power:**
  - A `computer` part has power inputs from any buses, through breakers.
  - It has a power-up time and a self-test, and it rides through short transients.
  - It resets when power is lost for longer than its hold-up time.
- **Health:** channel A/B or 1/2/3, with failures from its type (dead, frozen output, wrong data)
  and the developer's own failures added on top.
- **Data links:** typed links between computers (429-style labels, network-style messages), each
  with:
  - a source, a refresh rate and a latency;
  - a status (normal, no computed data, failure warning).

  The data goes stale when the sender dies, a switch fails or a bus drops.
- **The developer's code runs inside it:**
  - The developer registers a step function in C.
  - EasyAcft calls it only while the computer is powered and healthy, and hands it its inputs with
    their status.
  - EasyAcft publishes its outputs onto its links.
  - A dead computer's outputs go invalid on their own, so a failure reaches every system that
    reads them with no extra code.

The developer writes the logic; EasyAcft makes it fail and degrade like a real installation.

## EasyAcft Studio (GUI editor and visualiser)

A desktop app for Windows, macOS and Linux, written in Rust (egui). The TOML files stay the source
of truth: Studio reads and writes them, keeps comments and formatting, and anything edited by hand
shows up in Studio immediately. Teams can still diff and merge plain text.

**Look:**
- A plain, neutral UI: grey panels, one text face, no decorative colour.
- Diagrams are monochrome line drawings in manual-schematic style.
- State is shown by line weight, dash and text:
  - a solid heavy line is supplied;
  - a thin line is not supplied;
  - a dashed line is a fault;
  - every value is written next to its part.
- A developer can add colour through the theme file if they want it.

**Editing:**
- **Network editor:** each system is a node graph.
  - Drag parts from the catalogue and wire their ports.
  - Studio refuses a wire between the wrong port types.
  - Parameters show in a side panel with units.
- **Logic editor:** expressions with autocomplete for every name, plus live checker errors.
- **Engine editor:**
  - A side-view diagram of the gas path: add, remove and reorder stages, spools, bleeds and
    cooling flows.
  - Stage maps plotted with the running point on them.
  - The design point solves as you type.
  - `engine fit` with test points, showing the error per point.
- **FADEC and controller editor:** rating tables and schedules as editable plots, limiters and
  start sequences as timelines.
- **Templates and variants:** see what a variant file changes, side by side.

**Visualisation:**
- **Every network drawn live:** wires and pipes show supplied, not supplied or fault by line
  style, with values next to each part. Open and closed contactors and valves, and tripped breakers
  are shown.
- **Engine:** station table (pressure, temperature, flow at every station), a running line on
  each map, a T-s diagram and spool speeds.
- **Computers and data links:** which computers are powered and healthy, which links are stale,
  and their status bits.
- **Plots:** any value against time, several at once, with recording and replay from snapshots.
- **Failures:** click any part to fail it, schedule failures, or watch the maintenance log.

**Diagrams draw themselves:**
- **Networks:**
  - Studio lays out every network from the files with a layered layout using manual-schematic
    conventions: sources at the bottom, buses in rows by level, batteries and hot buses at the
    top, orthogonal wires.
  - Contactors and breakers sit on the wires they switch.
  - Any aircraft gets a readable diagram with no drawing work.
- **Engines:** the gas-path drawing is generated from the stage list. Each stage type knows how
  to draw itself at its own radius and stage count, and spools, bleeds, cooling flows and
  accessories come from their definitions. A turbojet, three-spool, geared fan or turboprop
  draws correctly from the same code.
- **Hand tweaks:**
  - A developer can drag anything, and the positions are saved in a small layout file next to
    the TOML.
  - The rest re-flows around pinned parts.
  - Adding a part never wrecks a hand-tuned diagram.
- **Detail levels:**
  - an overview: one block per system;
  - the network view;
  - a part's internals (a TR, a generator's drive).

  Labels and values hide or show with zoom.
- **State only restyles:** live values, faults and flows change line style and text, never the
  layout, so the diagram stays still while you watch it.
- **Same look in every view:** style comes from one theme file, the same in Studio, the in-sim
  window and exported images. A developer can restyle it, for example to their aircraft's own
  display colours.

**Where the live data comes from:**
- **Headless:** Studio runs the aircraft itself (`easyacft sim`) with scenario scripts and a time
  control (pause, step, 10x).
- **In the sim:** the EasyAcft plugin opens a local connection, and Studio attaches to the running
  X-Plane. Developers watch and edit live; saving a file hot-reloads it in the sim.

**In X-Plane itself:** a small built-in window (X-Plane's own window API) with the network
diagrams and the failure panel, for developers and testers who don't want a second app.

### Making Studio intuitive

1. **Start from something that works.**
   - New aircraft starts from a template: piston single, twin turboprop, twin jet or quad jet.
   - It flies and powers up in a minute, and the developer edits from there instead of facing a
     blank canvas.
2. **Ask "why?" about anything.**
   - Click a dead bus or a warning and Studio shows the chain of causes: AC BUS 2 is dead
     because GLC 2 is open, because GEN 2 has a fault, because drive oil pressure is low.
   - Each step is clickable. The compiled plan already knows every dependency, so this comes
     cheap.
3. **Always live.**
   - The aircraft runs while you edit, and every change applies instantly.
   - Everything can be undone, and a "changes since last save" list shows what moved.
4. **Touch it to try it.**
   - Click a contactor to switch it, flip cockpit switches on the diagram, drag a thrust lever.
   - Right-click any part to fail it.
5. **Plain words and real units.**
   - Every parameter has a one-line meaning and a typical range ("90 kVA generator: usually
     115 V, 400 Hz").
   - Presets for common parts.
   - Errors say what is wrong, with a button that applies the fix.
6. **Simple first, everything on demand.** The inspector shows the few settings most people
   change; "All settings" opens the rest.
7. **Search everything.** One command bar (Ctrl+K) finds parts, values, failures, datarefs and
   commands.
8. **Bind by dragging.**
   - Drag a value onto an X-Plane dataref, or a switch onto a cockpit click spot in the
     in-sim window.
   - The binding is written to the TOML.
9. **A health panel that nags kindly:**
   - unconnected ports;
   - loads above their rating;
   - buses that stay dead on a normal power-up;
   - switches bound to nothing;
   - failures nothing reads.
10. **Safe experiments.**
    - Try mode snapshots the aircraft, so you can test something and keep or discard it.
    - A time scrubber replays the last minutes.
11. **Record a test by flying it.** What you do in the sim or in Studio can be saved as a
    scenario with its checks, ready for CI.
12. **Help in place.** F1 on any part opens its docs. The example aircraft double as guided
    tours.
13. **Test it on real developers.** From M7, every milestone has 3-5 X-Plane developers try set
    tasks while we watch, and what confuses them gets fixed first.

## C API sketch

```c
easyacft_aircraft *easyacft_load(const char *dir, easyacft_diag *out);
void            easyacft_step(easyacft_aircraft *, double dt_s);
easyacft_handle    easyacft_find(easyacft_aircraft *, const char *path);   /* "dc1.voltage" */
double          easyacft_get(easyacft_aircraft *, easyacft_handle);
void            easyacft_set(easyacft_aircraft *, easyacft_handle, double);
int             easyacft_register_component(easyacft_aircraft *, const easyacft_component_def *);
size_t          easyacft_snapshot(easyacft_aircraft *, void *buf, size_t len);
```

The X-Plane layer (`easyacft-xp`) wraps this. It handles the flight loop, dataref and command binding,
and reloading files in the sim.

## Code layout

| Crate | Job |
|---|---|
| `easyacft-core` | Solver, scheduler, state, snapshots, units |
| `easyacft-parts` | Component catalogue per domain |
| `easyacft-lang` | TOML loader, expression compiler, diagnostics |
| `easyacft-check` | CLI: check, graph, fmt, explain, sim |
| `easyacft-xp` | X-Plane binding |
| `easyacft-ffi` | C ABI and the generated `easyacft.h` |
| `easyacft-studio` | GUI editor and visualiser; live link to `easyacft sim` or the running sim |

## Testing

- **Scenario files:** "at t=0 battery on, at t=5 APU start, expect dc1 above 27 V by t=40", run
  headless.
- **Golden runs:** the two proof aircraft run fixed scenarios, and their state digests are recorded.
- **CI:** Windows, macOS and Linux, plus a C example that links against `easyacft.h`.

## A380 example

`examples/a380/` in the EasyAcft repository is a backend-only A380 systems simulation:
- electrical, hydraulic, bleed, fuel, engines, APU, FADEC and failures;
- shown live in Studio, with no cockpit or displays.

- **Shipped as:** TOML files only, as an example built on EasyAcft.
- **Sources:**
  - public data: Airbus's A380 Aircraft Characteristics, EASA type certificates, the ICAO
    emissions databank, published papers and patents, accident reports;
  - facts checked against real A380 operating documentation.
- **Rule:** values and behaviour can come from any source. No manual text is pasted into comments
  or docs, and no scanned or copied manual pages are included. Diagrams are drawn by Studio from
  the example's own files.
- **Public wording:** "A380 systems simulation built on EasyAcft, checked against real A380 data."
- **Notice:** not affiliated with or endorsed by Airbus; A380 is Airbus's trademark.
- **Library rules unchanged:** EasyAcft's own source still follows the clean-room rules below, and
  the generic twin-jet and piston single stay the official examples.

## Clean-room rules

- **Separate repository**, starting from an empty tree.
- **Nothing from FlyByWire:** no code and no structure copied. That includes the A380 plugin's
  deep systems.
- **Nothing proprietary:** no FCOM, TTM or other Airbus/Boeing documents.
- **Allowed sources:** textbooks and public standards only, with each physics choice cited in its
  doc comment.
- **Similarity gate:** a script like `gate.py` checks for long token runs shared with FBW sources
  before every release.

## Milestones

| | Deliverable |
|---|---|
| M0 | Repo, units, solver core, segmented links, per-domain rates, snapshots, CI on 3 OS |
| M1 | Electrical parts, load shedding; `easyacft-lang` and `easyacft check` with diagnostics |
| M2 | X-Plane binding, cockpit switch behaviour, weight and balance; the piston single flies on EasyAcft electrical and fuel |
| M3 | Hydraulic, gear and brakes; pneumatic/bleed, pressurization, ice and rain, oxygen; fuel management |
| M4 | Configurable gas path, `easyacft engine fit`, forces to X-Plane, piston model, turboprop extras |
| M4b | FADEC, APU and its controller |
| M5 | Failures and breakers for every part; fire protection; lights; generic logic; sound events |
| M6 | Computers; air data and inertial sensors; maintenance log; wear and servicing |
| M7 | Studio viewer: live network and engine diagrams, plots, failures, headless runs and attach to X-Plane |
| M8 | Studio editor: network, logic, engine, FADEC and variant editing; in-sim window |
| M9 | Twin-jet proof aircraft complete; C API frozen at 1.0; docs; release |

## Performance

**Build time for developers:**
- EasyAcft ships prebuilt for Windows, macOS and Linux: a static library, a dynamic library and
  `easyacft.h`.
- A C/C++ developer never compiles Rust. Their own build is as fast as without EasyAcft.
- Aircraft files are not compiled into the plugin. The plugin reloads them in the sim in under a
  second, so most systems changes need no rebuild and no X-Plane restart.

**Run time:**
- **Compile once.** `easyacft check` turns the files into a flat plan:
  - names become indices, so there are no string lookups while flying;
  - expressions become bytecode with constants folded in;
  - parts nothing reads or binds are removed;
  - each network's equations are laid out once.

  The plan is cached as one binary file next to the TOML, so loading in the sim is instant.
- **Sparse, partial solves.** Each network splits into independent islands. An island whose
  switches, sources and loads didn't change since the last step is skipped.
- **Per-domain rates.** Slow systems (thermal, wear, fuel quantity) run a few times a second;
  fast ones run only as fast as their parts need.
- **Data-oriented layout.** Parts of the same type are stored together in arrays and stepped in
  a tight loop, which suits the CPU cache and auto-vectorisation.
- **Optional worker thread.** Islands that don't touch each other can step in parallel. Dataref
  reads and writes stay on X-Plane's main thread, batched once per frame.
- **Frame budget.**
  - `easyacft sim --profile` reports time per system and per part.
  - CI fails if the proof twin-jet takes more than a set budget per frame (target: under 0.3 ms
    on a mid-range CPU).

## Size

An estimate in Rust lines, about 40 % of them tests:

| Part | Lines |
|---|---|
| Core: solver, scheduler, units, snapshots, segments | 8-10k |
| Language: loader, expressions, templates, diagnostics | 10-12k |
| Tools: check, graph, fmt, explain, sim, scenarios, engine fit | 6-8k |
| Electrical, hydraulic, pneumatic, fuel parts | 15k |
| Gas path, piston, FADEC, APU | 18k |
| Computers, failures, breakers, maintenance log | 7k |
| The 15 ready-made systems | 15-20k |
| X-Plane binding, C API, header | 6-8k |
| Studio: viewer, editors, live link, in-sim window | 25-35k |
| **Total** | **about 110-135k** |

Not counted: the two proof aircraft (TOML files, about 5k lines) and the docs. For comparison,
libelec's own source is about 8k lines of C (electrical only); the rest of its repository is
bundled test tools and the X-Plane SDK, and it relies on libacfutils for everything else.

## Risks

- **Scope:** v1 is the whole aircraft. M2 already gives a usable electrical and fuel library, so
  there is something to release early if the rest slips.
- **Performance:** one general solver can be slower than hand-written code. The answer is sparse
  per-domain solves, sub-steps only where they are needed, and a frame budget test.
- **Expression language growth:** it must stay small. Anything bigger goes to the C API (or Lua, if
  you choose 2B).

## Status, 3 Nov 2026

M0 to M8 are built and tested; M9 (freeze, docs, release) is next.

Built as planned:
- every domain in the plan, the checker with error codes, units, expressions, variants (`[variant] of`, `remove`), scenario tests (`easyacft test`), import from `.acf` and MSFS;
- the X-Plane plugin with bindings, published values, switch commands, hot reload, the live link to Studio and the in-sim systems window;
- the C API with computers;
- three examples, each with scenario tests: twin-jet (184 parts), piston-single (52), a380 (300).

Studio, built differently from the plan:
- networks are edited through the inspector (typed settings, part pickers, live expression checks with completion) and by shift-dragging between two buses or lines; there is no port-wiring canvas;
- the engine editor is a side view drawn from the stages, the station values, and a design point that solves as you type; stage maps and the T-s diagram are not drawn;
- FADEC ratings and schedules are CSV tables, edited and plotted in Tables;
- one detail level (the network view); no overview or part internals yet;
- labels hide below 55 % zoom;
- the theme file `studio-theme.toml` sets four colours: ink, faint, fill, paper;
- the in-sim window is a list per system with fail and repair, not diagrams;
- binding is a form in the inspector, not drag and drop;
- recorded tests add each bus's powered state as the checks.

