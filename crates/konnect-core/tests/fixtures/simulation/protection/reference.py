"""Reference values for the protection decks, worked out with no SPICE engine.

Run with any Python 3. Every deck here is a resistive circuit with one memory
(the fuse's integral of i squared), so each value is the circuit equation solved
at a point, plus RK4 for the fuse. The live tests compare run_simulation with
what this prints.
"""
import math

VBAT, RSRC, RLOAD = 12.0, 20e-3, 12.0
T0, TAU, RON, ROFF = 5e-3, 5e-6, 5e-3, 1e9


def parallel(a, b):
    return a * b / (a + b)


def short(t):
    """The resistor across the output, falling from ROFF to RON."""
    s = 0.5 * (1 + math.tanh((t - T0) / TAU))
    return math.exp(math.log(ROFF) * (1 - s) + math.log(RON) * s)


def short_current(t):
    return VBAT / (RSRC + parallel(short(t), RLOAD))


def fuse_current(t, open_):
    ron, roff = 5e-3, 1e6
    fuse = roff if open_ else ron
    return VBAT / (RSRC + fuse + parallel(short(t), RLOAD))


def fuse(rated, stop):
    """Integrate i squared by RK4. The switch opens when it passes rated * 1.001."""
    trip = rated * 1.001
    # Until 4.9 ms the load current is constant, so start there.
    dt, t, opened_at = 1e-9, 4.9e-3, None
    q = t * fuse_current(0.0, False) ** 2
    while t < stop:
        open_ = q > trip
        if open_ and opened_at is None:
            opened_at = t
        # Before the short the current is the load current, constant.
        f = lambda tt: fuse_current(tt, open_) ** 2
        k1 = f(t)
        k2 = f(t + dt / 2)
        k4 = f(t + dt)
        q += dt * (k1 + 4 * k2 + k4) / 6
        t += dt
    return q, opened_at


def efuse_load(ilim=5.0, ron=20e-3):
    """The output voltage where the pass element and the load agree."""
    low, high = 0.0, VBAT
    for _ in range(200):
        vout = (low + high) / 2
        vds = ilim * ron * math.atanh(min(vout / RLOAD / ilim, 0.999999999))
        if VBAT - vout > vds:
            low = vout
        else:
            high = vout
    return low


def reverse(vbat, ron=30e-3, roff=12e6, knee=10e-3):
    """Battery voltage vbat in, through the stand-in, into the load."""

    def residual(vout):
        v = vbat - vout
        g = 1 / roff + (1 / ron - 1 / roff) * 0.5 * (1 + math.tanh(v / knee))
        return g * v - vout / RLOAD

    low, high = min(0.0, vbat), max(0.0, vbat)
    for _ in range(200):
        middle = (low + high) / 2
        if (residual(middle) > 0) == (residual(low) > 0):
            low = middle
        else:
            high = middle
    return low, low / RLOAD


def main():
    ron_parallel = parallel(RON, RLOAD)
    print("short before:   ", VBAT / (RSRC + RLOAD), "A")
    print("short settled:  ", VBAT / (RSRC + ron_parallel), "A")
    print("short vout:     ", VBAT * ron_parallel / (RSRC + ron_parallel), "V")

    q, opened_at = fuse(2.0, 7e-3)
    print("fuse before:    ", fuse_current(0.0, False), "A")
    print("fuse peak:      ", fuse_current(opened_at, False), "A, at the instant it opens")
    print("fuse opens at:  ", opened_at, "s")
    print("fuse let-through", q, "A2s (the rating is 2)")
    print("fuse after open:", fuse_current(9.9e-3, True), "A")
    q, opened_at = fuse(3000.0, 9.9e-3)
    print("fuse at 3000:   ", q, "A2s at 9.9 ms, opened at", opened_at)
    print("fuse held shut: ", fuse_current(9.9e-3, False), "A")

    vout = efuse_load()
    print("efuse load vout:", vout, "V,", vout / RLOAD, "A")
    print("efuse short:    ", 5.0, "A, vout", 5.0 * ron_parallel, "V")

    print("uvlo on, off:   ", 10.0 / 1000.0, "s,", 28e-3 - 9.0 / 1000.0, "s")
    print("uvlo out at 9.5 V rising:  ", 9.5 * RLOAD / (RLOAD + 1e9), "V")
    print("uvlo out at 9.5 V falling: ", 9.5 * RLOAD / (RLOAD + 10e-3), "V")

    vout, current = reverse(-12.0)
    print("reverse -12 V:  ", current, "A, vout", vout)
    vout, current = reverse(12.0)
    print("reverse +12 V:  ", current, "A, vout", vout)


if __name__ == "__main__":
    main()
