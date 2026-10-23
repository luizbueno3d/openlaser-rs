; OpenLaser G-code
; Machine: Diode laser · GRBL 400 × 400 mm
; Generated: <MASKED>
; Design origin: center; units: millimetres
G21
G90
M5
; Layer: Fast · vector_engrave
M5
G0 X190 Y190
M4 S122
G1 X210 Y190 F15000
G1 X210 Y210 F15000
G1 X190 Y210 F15000
G1 X190 Y190 F15000
M5
; Layer: Faint · vector_engrave
M5
G0 X200 Y200
M4 S0
G1 X210 Y200.001 F7.407
M5
; Layer: Faint2 · vector_engrave
M5
G0 X198.765 Y202.346
M4 S2
G1 X203.457 Y195.432 F8999.994
M5
M5
M9
G0 X0 Y0
; End OpenLaser job
