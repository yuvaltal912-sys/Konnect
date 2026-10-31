* Output short. Your circuit supplies the first three elements; the rest is the fault.
VBAT vbat 0 DC 12
RSRC vbat out 20m
RLOAD out 0 12
* Fault: a resistor across the output that falls from 1 G to 5 mohm around 5 ms.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
