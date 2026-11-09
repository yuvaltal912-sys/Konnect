# PCB layout checklist

The owner's standard (given 2026-11-08) for MCU, sensor, communication, power-supply and MOSFET-driver boards. Konnect's routing-quality and review work is measured against it: the circuit review (rank 29, #55) and the layout quality audit (#56) name the item each finding comes from.

Mark unused items N/A; take
exact widths, clearances, impedance and timing limits from the real parts, operating conditions
and stackup.

**Core principle: design every important signal or power path together with its return path.**
A short trace can still form a large, noisy current loop if its return takes a long route.

## 1. Requirements before layout
- Supply range, transients, continuous/peak currents, temperature range, mechanical constraints.
- Component ratings, logic-level compatibility, connector pinouts, footprint-to-datasheet pin numbering.
- Safe behaviour during power-up, reset, programming and loss of communication (MOSFETs, relays).
- The fab's stackup, copper thickness, minimum trace/space, drills, assembly capability.
- Separate routing rules for power, ordinary signals, sensitive analog, controlled impedance.
- Identify critical circuits and read their recommended layouts before placement.

## 2. Placement
- Outline, mounting holes, connectors, enclosure clearances first.
- Group by function: power entry, regulation, MCU, communication, analog measurement, power outputs.
- Arrange blocks by signal flow; rotate parts for direct connections.
- Separate switching stages and clocks from sensitive analog.
- Leave room for mating, programming, probing, soldering, rework.

| Component | Placement requirement |
|---|---|
| IC decoupling capacitor | Beside its supply pin, very short connection to both supply and ground |
| Local bulk capacitor | Near the circuit that draws the transient current |
| TVS/ESD protection | At the exposed connector, where the trace enters the board |
| CAN/RS-485 transceiver | Near its bus connector and protection |
| Series termination resistor | Near the signal's actual driver |
| Bus-end termination | At the physical bus endpoint |
| MOSFET gate driver / gate resistor | Driver near the MOSFETs; resistor beside its gate |
| Current-sense amplifier | Near the shunt, separate sense connections from its terminals |
| Crystal / ADC reference capacitor | Near the IC pins, per the device's layout guidance |

## 3. Stackup, ground, return paths
- Uninterrupted adjacent reference plane under critical routes.
- Enough layers to keep those references; on 2 layers keep one side largely continuous ground and inspect every cut.
- Signals must not cross plane splits, slots, voids or narrow ground bottlenecks.
- Continuous ground as the start; separate noisy and sensitive circuits by placement and current paths.
- Stitching via near a layer change between different reference planes.
- Ground pours actually connected; motor/relay/switching returns not sharing narrow paths with measurements.
- Keep intentional isolation barriers and antenna keepouts when adding copper or stitching vias.

## 4. Power distribution and decoupling
- Manufacturer's decoupling values and supply-pin connections.
- Minimise the whole capacitor → IC → ground → capacitor loop (check the ground side too).
- Local bulk capacitance for load transients.
- Capacitor voltage rating, effective capacitance under DC bias, ESR, ripple current.
- Size traces, pours and vias for current, voltage drop, copper thickness, temperature rise.
- Inspect bottlenecks: pads, neck-downs, thermal-relief spokes, connector contacts, layer transitions.
- Thermal copper and thermal vias for dissipating parts.

## 5. Routing and signal integrity
- Critical loops and constrained routes before ordinary GPIO.
- Direct routes, few branches, vias and detours.
- Judge transmission-line effects by rise/fall time, not only clock frequency.
- Reduce long parallel runs between unrelated signals; more spacing around clocks, switch nodes, sensitive inputs.
- Driver strength and slew rate for timing with little ringing.
- Source termination near the driver, value for that driver and interconnect.
- Controlled impedance from the actual fab stackup.
- Differential pairs: consistent spacing, reference and transitions; no branch stubs.
- Length matching only where timing/skew needs it; compact tuning.

## 6. Interfaces
- CAN: CANH/CANL together; linear bus, short stubs; 120 Ω at each physical end; selectable onboard termination if node position can change; ground reference / common mode / isolation.
- I²C: pull-ups from bus capacitance, rise time and sink capability; count module pull-ups; pull-up voltage and level shifting; check waveforms with the real cable.
- SPI: clean SCLK and its return; setup/hold incl. clock travel and data return; damping near the actual driver (MISO is driven by the peripheral).
- UART: TX/RX, levels, reference ground; a transceiver for long or noisy cables.
- USB 2.0: D+/D− at 90 Ω differential; device length/skew limits; low-capacitance ESD; no stubs.
- RS-485: topology and endpoint termination; fail-safe biasing; common mode, reference, isolation.

## 7. Analog, sensing, clocks, RF
- Low-level and high-impedance analog short and away from clocks, inductors, switch nodes.
- ADC input filters and reference decoupling near their pins; source impedance lets the ADC settle.
- Measurement returns so load-current drops do not become measurement error.
- Crystal per the oscillator's reference layout (placement, loading, grounding, keepouts).
- Wireless modules: antenna keepout on every specified layer; nearby metal, cables, enclosure.

## 8. Switching regulators, MOSFETs, inductive loads
- High-current paths in each switching state; minimise the loop carrying fast-changing current.
- Buck: the input loop is critical; boost: the output loop is critical.
- Compact switch-node copper within current/thermal needs; feedback and sensitive routes away from it.
- Gate drive with its own source/driver return; Kelvin connections where available.
- Bootstrap / charge-pump capacitors beside their driver pins.
- Kelvin sense traces from the shunt terminals; sense filter near the amplifier.
- Inductive recirculation path in every switching state, including disable and faults.
- H-bridges: protection for both polarities; energy returned to the supply.

## 9. External connections and protection
- Exposed signals pass the protection before the protected circuit.
- TVS/ESD with short low-inductance discharge paths; unprotected connector-side traces away from protected ones.
- Deliberate shield and chassis connections.
- Protection working voltage, clamping, leakage, capacitance, energy.
- Coordinate with reverse polarity, shorts, hot-plug, supply transients.
- Clearance and creepage for maximum voltage, transients, environment, standards.

## 10. Manufacturing and review before ordering
- Every footprint, pin 1, polarised part, connector orientation.
- Courtyards, mask openings, paste, drills, copper-to-edge.
- Thermal-pad vias and paste suit the assembly process.
- Programming access and test points (supplies, ground, reset, key signals); light loading on fast nets.
- Labels: connector function, polarity, supply voltage, revision.
- Refill zones; resolve ERC/DRC and document justified exceptions.
- Schematic-to-PCB consistency.
- Inspect exported manufacturing files (copper, mask, silk, drills, outline).
- 3D check for mounting, enclosure, connector interference.

## 11. Prototype validation
- Current-limited supply; verify rails before expensive loads.
- Supply droop, ripple, temperatures at maximum load.
- Critical signals at the receiver with a low-inductance probe connection.
- Communication with the longest cable, highest speed, all nodes.
- Communication and sensing while power stages and noisy loads run.
- Startup, reset, brownout, disconnected cables, fault behaviour.
- Record results against the requirements before calling the design validated.
