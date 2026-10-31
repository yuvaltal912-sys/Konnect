"""Reference values for soft_start.sp, worked out with no SPICE engine.

Run with any Python 3. It integrates the same circuit by classical RK4 at a
fixed step, solving the series path by bisection at each stage. The live test
compares run_simulation against the numbers this prints.
"""
VBAT, RPATH, VTO, KP, C, RLOAD, ISTEP = 12.0, 17.5e-3, 2.0, 100.0, 1000e-6, 12.0, 5.0
RAMP_START, RAMP_END, GATE = 10e-3, 11.25e-3, 20.0
STEP_START, STEP_EDGE, STEP_ON = 60e-3, 10e-6, 40e-3
DT = 1e-7


def gate(t):
    if t <= RAMP_START:
        return 0.0
    return min(GATE, GATE * (t - RAMP_START) / (RAMP_END - RAMP_START))


def load_step(t):
    t -= STEP_START
    if t <= 0:
        return 0.0
    if t < STEP_EDGE:
        return ISTEP * t / STEP_EDGE
    if t < STEP_EDGE + STEP_ON:
        return ISTEP
    if t < 2 * STEP_EDGE + STEP_ON:
        return ISTEP * (1 - (t - STEP_EDGE - STEP_ON) / STEP_EDGE)
    return 0.0


def drain_current(vds, vov):
    if vov <= 0 or vds <= 0:
        return 0.0
    if vds >= vov:
        return KP / 2 * vov * vov
    return KP * vds * (vov - vds / 2)


def path_current(t, vout):
    """The current where the path resistor and the MOSFET agree."""
    vov = gate(t) - vout - VTO
    low, high = 0.0, max(VBAT - vout, 0.0) / RPATH
    for _ in range(60):
        middle = (low + high) / 2
        if drain_current(VBAT - middle * RPATH - vout, vov) > middle:
            low = middle
        else:
            high = middle
    return low


def slope(t, vout):
    return (path_current(t, vout) - vout / RLOAD - load_step(t)) / C


def main():
    t, vout = 0.0, 0.0
    peak, peak_at, i2t, before, lowest, last_i = 0.0, 0.0, 0.0, 0.0, 1e9, 0.0
    while t < 140e-3:
        k1 = slope(t, vout)
        k2 = slope(t + DT / 2, vout + DT / 2 * k1)
        k3 = slope(t + DT / 2, vout + DT / 2 * k2)
        k4 = slope(t + DT, vout + DT * k3)
        vout += DT / 6 * (k1 + 2 * k2 + 2 * k3 + k4)
        t += DT
        current = path_current(t, vout)
        if t < STEP_START:
            i2t += DT * (current * current + last_i * last_i) / 2
            before = vout
        if current > peak:
            peak, peak_at = current, t
        if t > STEP_START:
            lowest = min(lowest, vout)
        last_i = current
    print(f"inrush peak {peak:.4f} A at {peak_at * 1e3:.4f} ms")
    print(f"i2t to 60 ms {i2t:.5f} A2s")
    print(f"rail before the step {before:.5f} V, lowest after {lowest:.5f} V")
    print(f"droop {(before - lowest) * 1e3:.3f} mV")


main()
