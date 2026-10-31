* eFuse into a net of the schematic. Exclude V1, D1 and R1 of the rectifier demo, and R2 and C1 are the load.
VBAT vbat GND DC 12
.param ef_ilim=5 ef_ron=20m
BEFZ vbat /rect_out I='ef_ilim*tanh(v(vbat,/rect_out)/(ef_ilim*ef_ron))'
