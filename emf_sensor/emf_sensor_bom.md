# EMF Mains-Presence Sensor — SMD Bill of Materials

Front end for the RCD controller's power sensor (see `220AC_EMF_Remote_detector.md` and
`emf_sensor_schematic.svg`). All parts chosen as **0603 / SOIC** surface-mount so the
board can be machine-assembled. LCSC part numbers are given for **JLCPCB SMT assembly**;
generic MPNs are given so the design ports to any assembler / distributor (Mouser, Digi-Key).

> ⚠️ **Verify the LCSC numbers and stock at order time.** JLCPCB's stock and the
> Basic/Extended classification change frequently; a part marked *Basic* today may be
> *Extended* (small per-part fee) tomorrow. The values, packages and dielectrics below are
> what matters — match those if a specific code is out of stock.

## Components

| Ref | Qty | Value | Package | Notes | MPN | LCSC | JLC type |
|-----|-----|-------|---------|-------|-----|------|----------|
| U1 | 1 | LM358 (dual op-amp) | SOIC-8 | One half used; tie off the other (see notes) | LM358DR | C7950 | Basic |
| R1, R2 | 2 | 10 kΩ 1% 1/10 W | 0603 | Virtual-ground divider | 0603WAF1002T5E | C25804 | Basic |
| R3 | 1 | 100 kΩ 1% | 0603 | IN− ↔ virtual ground (sets gain) | 0603WAF1003T5E | C25803 | Basic |
| R4 | 1 | 1 MΩ 1% | 0603 | Feedback (gain ≈ R4/R3 = 10) | 0603WAF1004T5E | C22935 | Basic |
| R5 | 1 | 100 Ω 1% | 0603 | Output / cable stabilization | 0603WAF1000T5E | C22775 | Basic |
| C1 | 1 | 100 nF 50 V X7R | 0603 | Antenna DC block | CL10B104KB8NNNC | C14663 | Basic |
| C2 | 1 | 100 nF 50 V X7R | 0603 | Virtual-ground bypass | CL10B104KB8NNNC | C14663 | Basic |
| C3 | 1 | 2.2 nF 50 V **C0G/NP0** (X7R ok) | 0603 | Feedback low-pass (~72 Hz band-limit) | CL10C222JB8NNNC | *verify* | Extended (likely) |
| C4 | 1 | 100 nF 50 V X7R | 0603 | VCC decoupling (recommended addition) | CL10B104KB8NNNC | C14663 | Basic |
| J1 | 1 | 4-pin 2.0 mm header, SMD | JST-PH SMD | Cable: 3V3 / GND / SIG / SHLD | S4B-PH-SM4-TB | *verify* | Extended |
| ANT | 1 | Antenna pad | — | Solder a 2–3" copper wire / castellated pad — not a purchased part | — | — | — |

**Consolidated buy:** the three 100 nF (C1, C2, C4) are the same part — order **3× C14663**.

## Assembly / layout notes

1. **Unused op-amp (U1B).** The LM358 is dual; only U1A is used. Tie off the spare to stop
   it oscillating: **pin 5 (+IN) → GND**, and **short pin 6 (−IN) ↔ pin 7 (OUT)**. No extra
   parts — just copper.
2. **C4 decoupling.** Place the 100 nF VCC cap physically close to pin 8 (VCC) ↔ pin 4 (GND).
   This was not in the original breadboard design but is standard practice on a PCB.
3. **C3 dielectric.** Prefer **C0G/NP0** for the feedback cap (stable, low distortion); X7R
   works but drifts more with temperature. The exact value isn't critical — 1–2.2 nF sets
   the ~72–160 Hz low-pass corner.
4. **Shield.** J1-SHLD is a pad with **no trace** on this board — the cable shield is grounded
   at the **ESP end only** (per the design doc) to avoid a ground loop.
5. **Antenna.** Keep the ANT pad and the U1 input (pin 2) traces short; that node is high-
   impedance and picks up noise. A short ground pour gap around it helps.

## Optional substitutions

* **Rail-to-rail op-amp** (more output headroom on 3.3 V than the LM358): **MCP6002-I/SN**
  (SOIC-8, LCSC **C7384** — *verify*). Pin-compatible; lets the 1.65 V bias swing further
  before clipping. Only needed if you raise the gain (R4 → 2.2 MΩ).
* **0402 passives** instead of 0603 if board space is tight — same values, change the
  package and pick the matching LCSC code.
* **Through-hole J1** (cheaper, hand-solderable): a 1×4 2.54 mm header, e.g. LCSC **C2337**.
