.title konnect simulation
.options savecurrents
V1 /in 0 PULSE(0 5 0 1u 1u 400u 800u)
R1 /in /out 1k
C1 /out 0 100n
M1 /out /in 0 0 nn
.model nn NMOS(VTO=1 KP=1e-3)
.tran 50u 1m
.end
