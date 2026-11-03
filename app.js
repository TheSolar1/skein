// Skein coverage map: French radio emitters (ANFR open data), terrain-aware reception range, antenna size needed.
const map = L.map("map", { preferCanvas: true }).setView([46.6, 2.5], 6);
L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 18, attribution: "© OpenStreetMap" }).addTo(map);

const $ = id => document.getElementById(id);
const ZOOM = 9, TILE = 512; // terrain tiles: about 100 m per pixel at France's latitude
const tileCache = new Map(), tileData = new Map(); // promises, and decoded pixels once loaded
let emitters = [], selected = null, layerCoverage = L.layerGroup().addTo(map), layerProbe = L.layerGroup().addTo(map);

// ── Terrain (Mapterhorn open elevation tiles: Terrarium encoding, 512 px tiles, CORS enabled) ──────────────────────────
const tileXY = (lat, lon, z) => {
  const n = 2 ** z, rad = (lat * Math.PI) / 180;
  return [((lon + 180) / 360) * n, ((1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2) * n];
};
function loadTile(x, y) {
  const key = `${x}/${y}`;
  if (!tileCache.has(key)) {
    tileCache.set(key, new Promise(res => {
      const img = new Image();
      img.crossOrigin = "anonymous";
      img.onload = () => {
        const c = document.createElement("canvas");
        c.width = c.height = TILE;
        const ctx = c.getContext("2d", { willReadFrequently: true });
        ctx.drawImage(img, 0, 0);
        const d = ctx.getImageData(0, 0, TILE, TILE).data;
        tileData.set(key, d);
        res(d);
      };
      img.onerror = () => res(null);
      img.src = `https://tiles.mapterhorn.com/${ZOOM}/${x}/${y}.webp`;
    }));
  }
  return tileCache.get(key);
}
async function prefetch(lat, lon, radiusKm) {
  const dLat = radiusKm / 111, dLon = radiusKm / (111 * Math.cos((lat * Math.PI) / 180));
  const [x0, y1] = tileXY(lat - dLat, lon - dLon, ZOOM), [x1, y0] = tileXY(lat + dLat, lon + dLon, ZOOM);
  const jobs = [];
  for (let x = Math.floor(x0); x <= Math.floor(x1); x++) for (let y = Math.floor(y0); y <= Math.floor(y1); y++) jobs.push(loadTile(x, y));
  await Promise.all(jobs);
}
// Synchronous elevation lookup once the tiles are prefetched (0 m if a tile is missing, e.g. at sea).
function elevation(lat, lon) {
  const [fx, fy] = tileXY(lat, lon, ZOOM);
  const tx = Math.floor(fx), ty = Math.floor(fy);
  const p = tileData.get(`${tx}/${ty}`);
  if (!p) return 0;
  const i = ((Math.floor((fy - ty) * TILE) * TILE) + Math.floor((fx - tx) * TILE)) * 4;
  return p[i] * 256 + p[i + 1] + p[i + 2] / 256 - 32768;
}

// ── Parameters ───────────────────────────────────────────────────────────────
const params = () => ({
  f: Number($("freq").value), eirp: Number($("eirp").value), gain: Number($("gain").value),
  rxH: Number($("rxh").value), sens: Number($("sens").value), maxKm: Number($("maxkm").value),
});
function fillFreqs(e) {
  const sel = $("freq"), bands = (e && e[4] ? e[4].split(",") : []).map(Number);
  sel.textContent = "";
  for (const b of [...new Set([...bands, 433, 868, 2400])].sort((a, b) => a - b))
    sel.add(new Option(`${b} MHz${[433, 868].includes(b) ? " (ISM, no licence)" : b === 2400 ? " (Wi-Fi/BLE, no licence)" : ""}`, b));
  sel.value = bands.length ? bands[0] : 868;
}

// ── Coverage ─────────────────────────────────────────────────────────────────
function profile(lat, lon, bearing, dKm, stepM) {
  const n = Math.round((dKm * 1000) / stepM), out = [];
  for (let i = 0; i <= n; i++) {
    const [la, lo] = Prop.destination(lat, lon, bearing, (i * stepM) / 1000);
    out.push(elevation(la, lo));
  }
  return out;
}
function rangeAlong(e, bearing, p, stepM = 250) {
  const prof = profile(e[0], e[1], bearing, p.maxKm, stepM);
  let last = 0;
  for (let i = 1; i < prof.length; i++) {
    const loss = Prop.pathLoss(prof.slice(0, i + 1), stepM, p.f, e[2] || 30, p.rxH);
    if (Prop.rxPower(p.eirp, p.gain, loss) < p.sens) break;
    last = i;
  }
  return (last * stepM) / 1000;
}
async function showCoverage(e) {
  const p = params();
  $("status").textContent = "Loading terrain…";
  await prefetch(e[0], e[1], p.maxKm);
  layerCoverage.clearLayers();
  const ring = [], free = [];
  let sum = 0, max = 0;
  // Free-space range for reference (no terrain).
  const freeKm = Math.min(p.maxKm, 10 ** ((p.eirp + p.gain - p.sens - 32.44 - 20 * Math.log10(p.f)) / 20));
  for (let b = 0; b < 360; b += 5) {
    const r = rangeAlong(e, b, p);
    sum += r; max = Math.max(max, r);
    ring.push(Prop.destination(e[0], e[1], b, Math.max(r, 0.05)));
    free.push(Prop.destination(e[0], e[1], b, freeKm));
  }
  L.polygon(free, { color: "#888", weight: 1, dashArray: "4 4", fill: false }).addTo(layerCoverage);
  L.polygon(ring, { color: "#e4572e", weight: 2, fillOpacity: 0.25 }).addTo(layerCoverage);
  const mean = sum / ring.length;
  $("info").innerHTML = `<b>${e[3] || "Unknown operator"}</b><br>Mast height ${e[2] || "?"} m · bands ${e[4] || "?"} MHz<br>` +
    `Range with terrain: <b>mean ${mean.toFixed(1)} km</b>, best ${max.toFixed(1)} km<br>Free-space range (no relief): ${freeKm.toFixed(1)} km<br>` +
    `<small>Orange = reachable with your receiver; dashed grey = no relief. Click anywhere on the map to get the antenna you need there.</small>`;
  $("status").textContent = "";
}

// Click anywhere: link budget from the selected emitter, with terrain.
async function probe(latlng) {
  if (!selected) return ($("status").textContent = "Select an emitter first (click a dot).");
  const p = params(), e = selected;
  const dKm = Prop.distKm(e[0], e[1], latlng.lat, latlng.lng);
  if (dKm > 150) return ($("status").textContent = "Too far (150 km max).");
  $("status").textContent = "Loading terrain…";
  await prefetch(e[0], e[1], Math.max(dKm, 1));
  const stepM = Math.max(50, Math.round((dKm * 1000) / 200));
  const n = Math.round((dKm * 1000) / stepM), prof = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    prof.push(elevation(e[0] + (latlng.lat - e[0]) * t, e[1] + (latlng.lng - e[1]) * t));
  }
  const loss = Prop.pathLoss(prof, (dKm * 1000) / n, p.f, e[2] || 30, p.rxH);
  const rx = Prop.rxPower(p.eirp, p.gain, loss), need = Prop.neededGain(p.eirp, loss, p.sens); // gain over a 0 dBi antenna
  const a = Prop.antennaSize(need, p.f), cm = m => (m < 1 ? `${(m * 100).toFixed(0)} cm` : `${m.toFixed(2)} m`);
  layerProbe.clearLayers();
  L.polyline([[e[0], e[1]], latlng], { color: "#2a7de1", weight: 2 }).addTo(layerProbe);
  L.marker(latlng).addTo(layerProbe).bindPopup(
    `<b>${dKm.toFixed(1)} km</b> from the emitter<br>Path loss ${loss.toFixed(0)} dB · signal ${rx.toFixed(0)} dBm with your antenna` +
    `<br><b>Antenna needed: ${need.toFixed(0)} dBi</b>` +
    (need === 0 ? `<br>A simple antenna is enough (half-wave dipole ${cm(a.dipole)}).` :
      `<br>Yagi ≈ ${a.yagiElements} elements, boom ${cm(a.yagiBoom)}<br>or dish ≈ ${cm(a.dishDiameter)} diameter`) +
    (need > 30 ? `<br><i>Above 30 dBi: unrealistic, relief blocks this path.</i>` : "")
  ).openPopup();
  $("status").textContent = "";
}

// ── Emitters layer ───────────────────────────────────────────────────────────
fetch("data/emitters.json").then(r => r.json()).then(list => {
  emitters = list;
  const cluster = L.markerClusterGroup({ chunkedLoading: true, disableClusteringAtZoom: 11, maxClusterRadius: 50 });
  cluster.addLayers(list.map(e => {
    const m = L.circleMarker([e[0], e[1]], { radius: 5, color: "#b00020", weight: 1, fillOpacity: 0.8 });
    m.on("click", ev => { L.DomEvent.stopPropagation(ev); selected = e; layerProbe.clearLayers(); fillFreqs(e); showCoverage(e); });
    return m;
  }));
  map.addLayer(cluster);
  $("count").textContent = `${list.length.toLocaleString("en")} emitters`;
});
map.on("click", ev => probe(ev.latlng));
for (const id of ["freq", "eirp", "gain", "rxh", "sens", "maxkm"]) $(id).onchange = () => selected && showCoverage(selected);
fillFreqs(null);
