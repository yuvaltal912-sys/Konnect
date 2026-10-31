* I2t fuse into a short. Your circuit supplies the supply, RSRC and the load.
VBAT vbat 0 DC 12
RSRC vbat fz_in 20m
RLOAD out 0 12
* Fault: the output short.
.param sh_t0=5m sh_tau=5u sh_ron=5m sh_roff=1G
.func sh_s() {0.5*(1+tanh((time-sh_t0)/sh_tau))}
RSHORT out 0 R='sh_roff**(1-sh_s())*sh_ron**sh_s()'
* Stand-in: a latching switch that opens once the integral of i^2 passes the rating.
.param fz_rated=2
VFZS fz_in fz_a 0
BFZQ 0 fz_q I='time>0 ? 1u*i(VFZS)*i(VFZS) : 0'
CFZQ fz_q 0 1u IC=0
RFZQ fz_q 0 1e12
BFZC fz_ctl 0 V='fz_rated-v(fz_q)'
SFZ fz_a out fz_ctl 0 SWFZ
.model SWFZ SW(Vt=0 Vh={fz_rated/1000} Ron=5m Roff=1Meg)
