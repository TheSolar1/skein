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


// lossProfile agrees with pathLoss at every sample.
const lp = P.lossProfile(ridge, 100, 868, 30, 2);
for (const i of [10, 50, 80, 100]) assert.ok(Math.abs(lp[i] - P.pathLoss(ridge.slice(0, i + 1), 100, 868, 30, 2)) < 0.01, `lossProfile[${i}]`);

// Gain classes: a dipole is class 0, 12 dBi class 3, 40 dBi unreachable.
assert.equal(P.gainClass(0), 0);
assert.equal(P.gainClass(12), 3);
assert.equal(P.gainClass(40), -1);


// antennaGain inverts antennaSize for Yagi and dish.
for (const g of [10, 15, 18]) assert.ok(Math.abs(P.antennaGain("yagi", P.antennaSize(g, 868).yagiBoom, 868) - g) < 1.5, `yagi ${g}`);
for (const g of [15, 24]) assert.ok(Math.abs(P.antennaGain("dish", P.antennaSize(g, 2400).dishDiameter, 2400) - g) < 0.01, `dish ${g}`);
assert.equal(P.antennaGain("dipole", 5, 868), 2.15);
assert.ok(P.antennaGain("omni", 1.4, 868) > P.antennaGain("omni", 0.35, 868));


// Directional antennas: no loss on the axis, 3 dB at half the beamwidth, strong rejection behind; omni has no pattern.
const yagi = { type: "yagi", size: 0.8, azimuth: 90 };
const gmax = P.antennaGain("yagi", 0.8, 868), bw = P.beamwidth("yagi", gmax);
assert.ok(bw > 20 && bw < 90, `beamwidth ${bw}`);
assert.equal(P.gainToward(yagi, 868, 90), gmax);
assert.ok(Math.abs(gmax - P.gainToward(yagi, 868, 90 + bw / 2) - 3) < 0.01);
assert.ok(gmax - P.gainToward(yagi, 868, 270) >= 24.9);
assert.equal(P.gainToward({ type: "omni", size: 1, azimuth: 0 }, 868, 123), P.antennaGain("omni", 1, 868));
assert.ok(Math.abs(P.bearing(45, 5, 46, 5)) < 0.01 && Math.abs(P.bearing(45, 5, 45, 6) - 90) < 1);

// Signal quality classes.
assert.equal(P.qualityClass(25), 0);
assert.equal(P.qualityClass(12), 1);
assert.equal(P.qualityClass(0.5), 3);
assert.equal(P.qualityClass(-1), -1);
console.log("propagation tests OK");
