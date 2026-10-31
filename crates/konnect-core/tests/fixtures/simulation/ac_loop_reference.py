"""Reference values for ac_loop.sp, worked out with no SPICE engine.

Run with any Python 3. The loop is L(f) = K / ((1 + jf/f1)(1 + jf/f2)(1 + jf/f3)), so every
number here is a root of a closed-form expression, found by bisection. The live test compares
run_simulation with what this prints.
"""
import math

K = 100.0
POLES = (100.0, 1_000.0, 10_000.0)


def gain(f):
    return K / math.prod(math.sqrt(1 + (f / p) ** 2) for p in POLES)


def phase(f):
    return -sum(math.degrees(math.atan(f / p)) for p in POLES)


def solve(low, high, falling):
    """The frequency where `falling` crosses zero, between two where it does not."""
    for _ in range(200):
        middle = math.sqrt(low * high)
        if falling(middle) > 0:
            low = middle
        else:
            high = middle
    return math.sqrt(low * high)


fc = solve(1.0, 1e6, lambda f: gain(f) - 1.0)
f180 = solve(1.0, 1e6, lambda f: phase(f) + 180.0)
corner = solve(1.0, 1e6, lambda f: 20 * math.log10(gain(f) / K) + 10 * math.log10(2))
print(f"crossover_hz       {fc:.6f}")
print(f"phase_margin       {180 + phase(fc):.6f}")
print(f"phase_crossover_hz {f180:.6f}")
print(f"gain_margin_db     {-20 * math.log10(gain(f180)):.6f}")
print(f"bandwidth_3db_hz   {corner:.6f}")
print(f"gain_db_at_1khz    {20 * math.log10(gain(1000.0)):.6f}")
print(f"phase_at_1khz      {phase(1000.0):.6f}")
