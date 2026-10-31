* Undervoltage lockout on a slow ramp. Your circuit supplies the input and the load.
VIN vin 0 PWL(0 0 14m 14 28m 0)
RLOAD out 0 12
* Stand-in: a switch that closes above uv_on and opens below uv_off, on its own input.
.param uv_on=10 uv_off=9
SUV vin out vin 0 SWUV
.model SWUV SW(Vt={(uv_on+uv_off)/2} Vh={(uv_on-uv_off)/2} Ron=10m Roff=1G)
