// Radio propagation maths (no DOM): free-space loss + single knife-edge diffraction over a terrain profile.
// Used by the map (app.js) and tested in Node (test/propagation.test.js).
(function (root) {
  const C = 299792458;
  const R_EARTH = 6371000 * 4 / 3; // effective earth radius (k = 4/3)

  const toRad = d => (d * Math.PI) / 180;
  const wavelength = fMHz => C / (fMHz * 1e6);

  // Free-space path loss in dB.
  const fspl = (dKm, fMHz) => 20 * Math.log10(Math.max(dKm, 0.001)) + 20 * Math.log10(fMHz) + 32.44;

  // Knife-edge diffraction loss (ITU-R P.526 approximation) for Fresnel parameter v.
  const knifeEdge = v => (v <= -0.78 ? 0 : 6.9 + 20 * Math.log10(Math.sqrt((v - 0.1) ** 2 + 1) + v - 0.1));

  // Haversine distance (km) and initial bearing helpers.
  function distKm(lat1, lon1, lat2, lon2) {
    const dLat = toRad(lat2 - lat1), dLon = toRad(lon2 - lon1);
    const a = Math.sin(dLat / 2) ** 2 + Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLon / 2) ** 2;
    return 2 * 6371 * Math.asin(Math.sqrt(a));
  }
  function destination(lat, lon, bearingDeg, dKm) {
    const d = dKm / 6371, b = toRad(bearingDeg), p1 = toRad(lat), l1 = toRad(lon);
    const p2 = Math.asin(Math.sin(p1) * Math.cos(d) + Math.cos(p1) * Math.sin(d) * Math.cos(b));
    const l2 = l1 + Math.atan2(Math.sin(b) * Math.sin(d) * Math.cos(p1), Math.cos(d) - Math.sin(p1) * Math.sin(p2));
    return [(p2 * 180) / Math.PI, (((l2 * 180) / Math.PI + 540) % 360) - 180];
  }

  // Path loss (dB) along a terrain profile.
  // profile: elevations (m) sampled every stepM metres from the transmitter (index 0) to the receiver (last index).
  // txH / rxH: antenna heights above ground (m).
  function pathLoss(profile, stepM, fMHz, txH, rxH) {
    const n = profile.length - 1;
    const D = n * stepM;
    if (n < 1) return 0;
    const lam = wavelength(fMHz);
    const h0 = profile[0] + txH, h1 = profile[n] + rxH;
    let worst = -Infinity;
    for (let i = 1; i < n; i++) {
      const d1 = i * stepM, d2 = D - d1;
      const bulge = (d1 * d2) / (2 * R_EARTH); // earth curvature
      const los = h0 + ((h1 - h0) * d1) / D;
      const h = profile[i] + bulge - los; // obstruction above the line of sight
      const v = h * Math.sqrt((2 / lam) * (D / (d1 * d2)));
      if (v > worst) worst = v;
    }
    return fspl(D / 1000, fMHz) + (worst > -Infinity ? knifeEdge(worst) : 0);
  }

  // Received power in dBm.
  const rxPower = (eirpDbm, rxGainDbi, lossDb) => eirpDbm + rxGainDbi - lossDb;

  // Antenna gain (dBi) needed to reach `sensDbm`, given the loss. 0 if a bare antenna is enough.
  const neededGain = (eirpDbm, lossDb, sensDbm) => Math.max(0, sensDbm - (eirpDbm - lossDb));

  // Physical size of an antenna giving `gainDbi` at `fMHz` (rough rules of thumb, in metres).
  function antennaSize(gainDbi, fMHz) {
    const lam = wavelength(fMHz);
    const g = 10 ** (gainDbi / 10);
    return {
      dipole: lam / 2, // half-wave dipole, ~2.15 dBi
      quarterWave: lam / 4,
      // Yagi: gain ~ 7.5 + 2.5*log2(elements/3)... approximated: elements = 3 * 2^((gain-7.5)/2.5); boom ~ 0.4 lambda per element.
      yagiElements: gainDbi <= 7.5 ? 3 : Math.min(60, Math.round(3 * 2 ** ((gainDbi - 7.5) / 2.5))),
      yagiBoom: (gainDbi <= 7.5 ? 3 : Math.min(60, Math.round(3 * 2 ** ((gainDbi - 7.5) / 2.5)))) * 0.4 * lam,
      // Parabolic dish with 55% efficiency: G = eta * (pi*D/lam)^2.
      dishDiameter: (lam / Math.PI) * Math.sqrt(g / 0.55),
    };
  }

  const api = { C, wavelength, fspl, knifeEdge, distKm, destination, pathLoss, rxPower, neededGain, antennaSize };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.Prop = api;
})(typeof self !== "undefined" ? self : this);
