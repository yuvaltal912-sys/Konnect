* 12 V rail: a MOSFET ramps 1000 uF up, then a 5 A load steps on at 60 ms.
VBAT vbat 0 DC 12
RPATH vbat drain 17.5m
M1 drain gate vout vout NSW
.model NSW NMOS(LEVEL=1 VTO=2 KP=100)
VGATE gate 0 PWL(0 0 10m 0 11.25m 20)
CLOAD vout 0 1000u
RLOAD vout 0 12
ILOAD vout 0 PULSE(0 5 60m 10u 10u 40m 1)
