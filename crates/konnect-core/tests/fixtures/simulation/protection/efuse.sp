* eFuse current limit into a short. Your circuit supplies the supply and the load.
VBAT vbat 0 DC 12
RLOAD out 0 12
* Fault: the output short.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
* Stand-in: a pass element that is ef_ron below the limit and holds ef_ilim above it.
.param ef_ilim=5 ef_ron=20m
BEFZ vbat out I='ef_ilim*tanh(v(vbat,out)/(ef_ilim*ef_ron))'
