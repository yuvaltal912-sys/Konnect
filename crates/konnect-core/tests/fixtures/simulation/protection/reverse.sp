* Reversed battery on a slow ramp from -12 V to 12 V. Your circuit supplies the battery and the load.
VBAT vbat 0 PWL(0 -12 24m 12)
RLOAD out 0 12
* Stand-in: a pass element that conducts forward at rb_ron and leaks backward through rb_roff.
.param rb_ron=30m rb_roff=12Meg rb_knee=10m
BRB vbat out I='v(vbat,out)*(1/rb_roff+(1/rb_ron-1/rb_roff)*0.5*(1+tanh(v(vbat,out)/rb_knee)))'
