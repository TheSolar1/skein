const assert = require("node:assert/strict");
const P = require("../propagation.js");

// FSPL at 1 km / 1000 MHz is about 92.4 dB.
assert.ok(Math.abs(P.fspl(1, 1000) - 92.44) < 0.05);
// Doubling the distance adds 6 dB.
assert.ok(Math.abs(P.fspl(2, 868) - P.fspl(1, 868) - 6.02) < 0.01);

// Flat terrain: close to free space (a few dB of Fresnel-zone grazing at most).
const flat = new Array(101).fill(100);
const lf = P.pathLoss(flat, 100, 868, 30, 2);
assert.ok(Math.abs(lf - P.fspl(10, 868)) < 5, `flat loss ${lf}`);

// A 200 m ridge in the middle of a 10 km path adds a lot of loss.
const ridge = flat.slice();
for (let i = 45; i <= 55; i++) ridge[i] = 300;
assert.ok(P.pathLoss(ridge, 100, 868, 30, 2) > lf + 15);

// Needed gain: signal at -130 dBm with -120 dBm sensitivity needs 10 dBi.
assert.equal(P.neededGain(50, 180, -120), 10);
assert.equal(P.neededGain(50, 120, -120), 0);

// Antenna sizes: half-wave dipole at 868 MHz is ~17 cm; 24 dBi dish at 3.5 GHz is well under 1 m.
const s = P.antennaSize(3, 868);
assert.ok(Math.abs(s.dipole - 0.1727) < 0.001);
assert.ok(P.antennaSize(24, 3500).dishDiameter < 0.6);

// Destination round trip: 100 km north then back.
const [lat, lon] = P.destination(48, 2, 0, 100);
assert.ok(Math.abs(P.distKm(48, 2, lat, lon) - 100) < 0.01);
console.log("propagation tests OK");
