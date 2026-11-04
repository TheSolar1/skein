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

  // Path loss to every sample of the profile (index i = receiver at i*stepM). Same model as pathLoss, O(n^2) total.
  function lossProfile(profile, stepM, fMHz, txH, rxH) {
    const lam = wavelength(fMHz), out = new Float32Array(profile.length), h0 = profile[0] + txH;
    for (let i = 1; i < profile.length; i++) {
      const D = i * stepM, h1 = profile[i] + rxH;
      let worst = -Infinity;
      for (let j = 1; j < i; j++) {
        const d1 = j * stepM, d2 = D - d1;
        const h = profile[j] + (d1 * d2) / (2 * R_EARTH) - (h0 + ((h1 - h0) * d1) / D);
        const v = h * Math.sqrt((2 / lam) * (D / (d1 * d2)));
        if (v > worst) worst = v;
      }
      out[i] = fspl(D / 1000, fMHz) + (worst > -Infinity ? knifeEdge(worst) : 0);
    }
    return out;
  }

  // Antenna classes for the colour-coded zones: the smallest antenna (by gain over an isotropic radiator) that hears the signal.
  const GAIN_CLASSES = [
    { max: 2.15, color: "#2ecc71", label: "Stock antenna (dipole / whip)" },
    { max: 6, color: "#a4d916", label: "Omni 5 dBi" },
    { max: 10, color: "#f4d03f", label: "Small Yagi" },
    { max: 15, color: "#f39c12", label: "Medium Yagi" },
    { max: 20, color: "#e74c3c", label: "Long Yagi or small dish" },
    { max: 30, color: "#8e44ad", label: "Large dish" },
  ];
  const gainClass = g => GAIN_CLASSES.findIndex(c => g <= c.max); // -1 = out of reach

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
      // Yagi: gain ~ 7 + 3.5*log2(elements/3) dBi (3 el = 7, 7 el = 11, 15 el = 15, 30 el = 18.6), so elements = 3 * 2^((gain-7)/3.5); boom ~ 0.4 lambda per element.
      yagiElements: gainDbi <= 7 ? 3 : Math.min(60, Math.round(3 * 2 ** ((gainDbi - 7) / 3.5))),
      yagiBoom: (gainDbi <= 7 ? 3 : Math.min(60, Math.round(3 * 2 ** ((gainDbi - 7) / 3.5)))) * 0.4 * lam,
      // Parabolic dish with 55% efficiency: G = eta * (pi*D/lam)^2.
      dishDiameter: (lam / Math.PI) * Math.sqrt(g / 0.55),
    };
  }

  // Antenna types the map lets you pick. `size` is the physical dimension in metres (see `sizeLabel`).
  const ANTENNAS = {
    dipole: { label: "Dipole / whip", sizeLabel: "fixed (half wave)" },
    omni: { label: "Omni collinear", sizeLabel: "length (m)" },
    yagi: { label: "Yagi", sizeLabel: "boom length (m)" },
    panel: { label: "Patch panel", sizeLabel: "side (m)" },
    dish: { label: "Dish", sizeLabel: "diameter (m)" },
  };
  // Gain (dBi) of an antenna of the given type and size at fMHz. Rule-of-thumb formulas, inverse of antennaSize for Yagi and dish.
  function antennaGain(type, size, fMHz) {
    const lam = wavelength(fMHz), s = Math.max(size || 0, 0);
    switch (type) {
      case "omni": return Math.min(12, 2.15 + 3 * Math.log2(Math.max(s / (lam / 2), 1)));
      case "yagi": {
        const n = Math.min(60, Math.max(3, Math.round(s / (0.4 * lam))));
        return 7 + 3.5 * Math.log2(n / 3);
      }
      case "panel": return Math.min(20, Math.max(2.15, 10 * Math.log10((4 * Math.PI * s * s * 0.7) / (lam * lam))));
      case "dish": return Math.max(2.15, 10 * Math.log10(0.55 * (Math.PI * s / lam) ** 2));
      default: return 2.15;
    }
  }

  const api = { ANTENNAS, antennaGain, C, wavelength, fspl, knifeEdge, distKm, destination, pathLoss, lossProfile, GAIN_CLASSES, gainClass, rxPower, neededGain, antennaSize };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.Prop = api;
})(typeof self !== "undefined" ? self : this);
