// Skein map: mesh nodes (Skein + Meshtastic) and the zones where you can receive them, coloured by the antenna you need.
const map = L.map("map", { preferCanvas: true }).setView([46.6, 2.5], 6);
L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", { maxZoom: 18, attribution: "© OpenStreetMap" }).addTo(map);

const $ = id => document.getElementById(id);
const ZOOM = 9, TILE = 512; // terrain tiles (Mapterhorn, Terrarium encoding): about 100 m per pixel at France's latitude
const CELL = 300, STEP = 200, RAY = 0.4; // zone grid cell (m), terrain sample spacing (m), angle between radials (deg)
const MAX_CELLS = 700 * 700, MAX_NODES = 60;
const ROUTER_ROLES = new Set(["ROUTER", "ROUTER_LATE", "ROUTER_CLIENT", "REPEATER"]);
const PRESETS = { 868: { eirp: 14, sens: -130 }, 433: { eirp: 10, sens: -130 }, 2400: { eirp: 10, sens: -95 } }; // LoRa EU868 / LoRa 433 / Wi-Fi-BLE

const tileCache = new Map(), tileData = new Map();
const store = (k, v) => { try { v === undefined ? JSON.parse(localStorage.getItem(k)) : localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} };
let linking = false, linkA = null, linkLayer = L.layerGroup().addTo(map), skeinNodes = [], meshNodes = [], masts = null, zoneLayer = null, grid = null, selectedMarker = null, adding = false;
const layers = { skein: L.layerGroup().addTo(map), mesh: L.layerGroup().addTo(map), masts: L.layerGroup() };

// ── Terrain ──────────────────────────────────────────────────────────────────
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
        tileData.set(key, ctx.getImageData(0, 0, TILE, TILE).data);
        res();
      };
      img.onerror = () => res();
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
// Elevation in metres once the tiles are prefetched (0 where a tile is missing, e.g. at sea).
function elevation(lat, lon) {
  const [fx, fy] = tileXY(lat, lon, ZOOM), tx = Math.floor(fx), ty = Math.floor(fy);
  const p = tileData.get(`${tx}/${ty}`);
  if (!p) return 0;
  const i = (Math.floor((fy - ty) * TILE) * TILE + Math.floor((fx - tx) * TILE)) * 4;
  return p[i] * 256 + p[i + 1] + p[i + 2] / 256 - 32768;
}

// ── Parameters ───────────────────────────────────────────────────────────────
const params = () => ({
  f: Number($("freq").value), eirp: Number($("eirp").value), sens: Number($("sens").value),
  nodeGain: Number($("ngain").value), rxH: Number($("rxh").value), maxKm: Number($("maxkm").value), txH: Number($("txh").value),
});
const status = t => ($("status").textContent = t);
const tick = () => new Promise(r => setTimeout(r, 0));

// ── Zones: for every grid cell, the smallest antenna gain that still hears one of the nodes ──
async function computeZones(nodes, p) {
  if (!nodes.length) return status("No node to compute: turn a layer on and zoom on an area with nodes.");
  const crs = map.options.crs, sec = lat => 1 / Math.cos((lat * Math.PI) / 180);
  const lat0 = nodes.reduce((s, n) => s + n.lat, 0) / nodes.length, cellP = CELL * sec(lat0), reach = p.maxKm * 1000 * sec(lat0);
  const pts = nodes.map(n => crs.project(L.latLng(n.lat, n.lon)));
  const x0 = Math.min(...pts.map(q => q.x)) - reach, x1 = Math.max(...pts.map(q => q.x)) + reach;
  const y0 = Math.min(...pts.map(q => q.y)) - reach, y1 = Math.max(...pts.map(q => q.y)) + reach;
  const W = Math.ceil((x1 - x0) / cellP), H = Math.ceil((y1 - y0) / cellP);
  if (W * H > MAX_CELLS) return status(`Area too large (${W}×${H} cells): zoom in or reduce the max distance.`);
  const need = new Float32Array(W * H).fill(Infinity);
  for (let k = 0; k < nodes.length; k++) {
    const n = nodes[k];
    status(`Computing zones… node ${k + 1}/${nodes.length}`);
    await prefetch(n.lat, n.lon, p.maxKm);
    await tick();
    const steps = Math.round((p.maxKm * 1000) / STEP), prof = new Float32Array(steps + 1), at = new Array(steps + 1);
    for (let b = 0; b < 360; b += RAY) {
      for (let i = 0; i <= steps; i++) {
        at[i] = Prop.destination(n.lat, n.lon, b, (i * STEP) / 1000);
        prof[i] = elevation(at[i][0], at[i][1]);
      }
      const loss = Prop.lossProfile(prof, STEP, p.f, p.txH, p.rxH);
      for (let i = 1; i <= steps; i++) {
        const g = Math.max(0, p.sens - (p.eirp - loss[i])); // gain over 0 dBi needed at this point
        const q = crs.project(L.latLng(at[i][0], at[i][1]));
        const cx = Math.floor((q.x - x0) / cellP), cy = Math.floor((q.y - y0) / cellP);
        if (cx >= 0 && cx < W && cy >= 0 && cy < H && g < need[cy * W + cx]) need[cy * W + cx] = g;
      }
    }
  }
  // Paint: one pixel per cell, north up.
  const cv = document.createElement("canvas");
  cv.width = W; cv.height = H;
  const ctx = cv.getContext("2d"), img = ctx.createImageData(W, H);
  const rgb = Prop.GAIN_CLASSES.map(c => [1, 3, 5].map(i => parseInt(c.color.slice(i, i + 2), 16)));
  const count = new Array(rgb.length).fill(0);
  for (let cy = 0; cy < H; cy++) for (let cx = 0; cx < W; cx++) {
    const c = Prop.gainClass(need[cy * W + cx]);
    if (c < 0) continue;
    count[c]++;
    const o = ((H - 1 - cy) * W + cx) * 4;
    img.data.set([...rgb[c], 150], o);
  }
  ctx.putImageData(img, 0, 0);
  const sw = crs.unproject(L.point(x0, y0)), ne = crs.unproject(L.point(x0 + W * cellP, y0 + H * cellP));
  if (zoneLayer) map.removeLayer(zoneLayer);
  zoneLayer = L.imageOverlay(cv.toDataURL(), L.latLngBounds(sw, ne), { opacity: 1, interactive: false }).addTo(map);
  grid = { need, W, H, x0, y0, cellP, crs, f: p.f, nodes: nodes.length };
  const km2 = count.map(n => (n * CELL * CELL) / 1e6);
  $("zonestats").innerHTML = Prop.GAIN_CLASSES.map((c, i) => `<tr><td><i style="background:${c.color}"></i>${c.label}</td><td>${Math.round(km2[i]).toLocaleString("en")} km²</td></tr>`).join("");
  status(`Zones from ${nodes.length} node${nodes.length > 1 ? "s" : ""}. Click anywhere to read the antenna needed.`);
  legend(p.f);
}

function legend(f) {
  $("legend").innerHTML = Prop.GAIN_CLASSES.map(c => {
    const a = Prop.antennaSize(c.max, f), cm = m => (m < 1 ? `${Math.round(m * 100)} cm` : `${m.toFixed(1)} m`);
    const size = c.max <= 2.15 ? `${cm(a.dipole)} dipole` : c.max <= 6 ? `~${cm(a.dipole * 3)} omni` : c.max <= 15 ? `Yagi ${a.yagiElements} el., ${cm(a.yagiBoom)}` : `dish ${cm(a.dishDiameter)}`;
    return `<div><i style="background:${c.color}"></i><b>≤ ${c.max} dBi</b> ${c.label}<small>${size}</small></div>`;
  }).join("");
}

// ── Nodes ────────────────────────────────────────────────────────────────────
const nodeIcon = (color, r) => ({ radius: r, color: "#fff", weight: 1.5, fillColor: color, fillOpacity: 0.95 });
function visibleNodes() {
  const mode = $("zonesrc").value, b = map.getBounds(), inView = n => b.contains([n.lat, n.lon]);
  const skein = skeinNodes.filter(inView), mesh = meshNodes.filter(inView);
  const routers = mesh.filter(n => ROUTER_ROLES.has(n.role));
  const pick = { skein, routers, mesh, all: [...skein, ...mesh] }[mode];
  return [...pick].sort((a, b) => ROUTER_ROLES.has(b.role) - ROUTER_ROLES.has(a.role)).slice(0, MAX_NODES);
}
function select(n, marker) {
  if (selectedMarker) selectedMarker.setStyle({ weight: 1.5 });
  selectedMarker = marker; marker.setStyle({ weight: 4 });
  $("info").innerHTML = `<b>${n.kind === "skein" ? "Skein node" : "Meshtastic node"}</b>` +
    `<br>${n.role || ""}${n.hw ? " · " + n.hw : ""}<br>${n.lat.toFixed(4)}, ${n.lon.toFixed(4)}${n.h ? ` · antenna ${n.h} m` : ""}`;
  const p = params();
  computeZones([n], { ...p, txH: n.h || p.txH });
}
function drawNodes() {
  layers.skein.clearLayers(); layers.mesh.clearLayers();
  const add = (n, group, color, r) => {
    const m = L.circleMarker([n.lat, n.lon], nodeIcon(color, r)).addTo(group);
    m.on("click", ev => { L.DomEvent.stopPropagation(ev); linking ? pickLink(n) : select(n, m); });
  };
  const onlyRouters = $("routers").checked;
  for (const n of meshNodes) if (!onlyRouters || ROUTER_ROLES.has(n.role)) add(n, layers.mesh, ROUTER_ROLES.has(n.role) ? "#1565c0" : "#5b9bd5", ROUTER_ROLES.has(n.role) ? 6 : 4);
  for (const n of skeinNodes) add(n, layers.skein, "#00a86b", 8);
  $("counts").textContent = `${skeinNodes.length} Skein · ${meshNodes.length.toLocaleString("en")} Meshtastic`;
}
const localSkein = () => store("skein_nodes_local") || [];

// ── Click on the map: read the zone grid, or place a node ────────────────────
map.on("click", ev => {
  if (linking) return pickLink({ lat: ev.latlng.lat, lon: ev.latlng.lng, kind: "point", role: "POINT" });
  if (adding) {
    adding = false; $("add").classList.remove("on"); map.getContainer().style.cursor = "";
    const mine = localSkein(), n = { lat: +ev.latlng.lat.toFixed(5), lon: +ev.latlng.lng.toFixed(5), h: Number($("txh").value), kind: "skein", role: "SKEIN" };
    mine.push(n); store("skein_nodes_local", mine); skeinNodes.push(n); drawNodes();
    return status("Node added (saved in this browser). Use “Export my nodes” to share them.");
  }
  if (!grid) return status("Click a node, or press “Zones for nodes in view”, first.");
  const q = grid.crs.project(ev.latlng), cx = Math.floor((q.x - grid.x0) / grid.cellP), cy = Math.floor((q.y - grid.y0) / grid.cellP);
  const g = cx >= 0 && cx < grid.W && cy >= 0 && cy < grid.H ? grid.need[cy * grid.W + cx] : Infinity, c = Prop.gainClass(g);
  const a = Prop.antennaSize(g, grid.f), cm = m => (m < 1 ? `${Math.round(m * 100)} cm` : `${m.toFixed(1)} m`);
  L.popup().setLatLng(ev.latlng).setContent(c < 0
    ? "<b>Out of reach</b><br>No antenna up to 30 dBi hears these nodes here (terrain or distance)."
    : `<b>${Prop.GAIN_CLASSES[c].label}</b><br>needs ≈ ${g.toFixed(1)} dBi` +
      (g > 2.15 ? `<br>Yagi ≈ ${a.yagiElements} elements, boom ${cm(a.yagiBoom)}<br>or dish ≈ ${cm(a.dishDiameter)}` : `<br>A ${cm(a.dipole)} dipole is enough.`)).openOn(map);
});

// ── Link check: can two nodes talk to each other? ────────────────────────────
function pickLink(n) {
  if (!linkA) {
    linkA = n; linkLayer.clearLayers();
    L.circleMarker([n.lat, n.lon], { radius: 10, color: "#ff2d95", weight: 3, fill: false }).addTo(linkLayer);
    return status("Now click the second node (or any point).");
  }
  const a = linkA; linkA = null;
  setLinking(false);
  checkLink(a, n, params());
}
function setLinking(on) {
  linking = on; linkA = null;
  $("link").classList.toggle("on", on);
  map.getContainer().style.cursor = on ? "crosshair" : "";
  if (on) { linkLayer.clearLayers(); status("Click the first node (or any point)."); }
}
const fmtM = m => (m < 1 ? `${Math.round(m * 100)} cm` : `${m.toFixed(1)} m`);
async function checkLink(a, b, p) {
  const dKm = Prop.distKm(a.lat, a.lon, b.lat, b.lon);
  if (dKm < 0.05) return status("These two points are the same place.");
  if (dKm > 200) return status("Too far apart (200 km max).");
  status("Loading terrain…");
  await prefetch((a.lat + b.lat) / 2, (a.lon + b.lon) / 2, dKm / 2 + 2);
  const step = Math.max(STEP, (dKm * 1000) / 600), n = Math.round((dKm * 1000) / step), prof = new Float32Array(n + 1);
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    prof[i] = elevation(a.lat + (b.lat - a.lat) * t, a.lon + (b.lon - a.lon) * t);
  }
  const hA = a.h || p.txH, hB = b.h || p.txH, stepM = (dKm * 1000) / n;
  const loss = Math.max(Prop.pathLoss(prof, stepM, p.f, hA, hB), Prop.pathLoss(Array.from(prof).reverse(), stepM, p.f, hB, hA)); // worst direction
  const rx = p.eirp + p.nodeGain - loss, margin = rx - p.sens;
  const free = Prop.fspl(dKm, p.f), extra = loss - free; // extra = terrain obstruction loss
  const ok = margin >= 0, miss = Math.max(0, -margin);
  const need = Prop.antennaSize(miss / 2 + p.nodeGain, p.f); // gain split between both ends
  $("linkres").innerHTML = `<b style="color:${ok ? "#1e9e5a" : "#d33"}">${ok ? "✔ Connected" : "✘ Not connected"}</b> · ${dKm.toFixed(1)} km<br>` +
    `Signal ${rx.toFixed(0)} dBm (needs ${p.sens} dBm) → margin <b>${margin >= 0 ? "+" : ""}${margin.toFixed(1)} dB</b><br>` +
    `Loss ${loss.toFixed(0)} dB = free space ${free.toFixed(0)} dB + terrain ${Math.max(0, extra).toFixed(0)} dB<br>` +
    (extra < 3 ? "Clear line of sight." : extra < 12 ? "Partly blocked by the terrain." : "Blocked by the terrain.") +
    (ok ? "" : `<br>Missing ${miss.toFixed(0)} dB: with a ≈ ${(miss / 2 + p.nodeGain).toFixed(0)} dBi antenna at <i>each</i> end (Yagi ≈ ${need.yagiElements} el., ${fmtM(need.yagiBoom)}) it would work` +
      (miss / 2 + p.nodeGain > 30 ? ", but that is unrealistic: raise the antennas or add a relay in between." : "."));
  linkLayer.clearLayers();
  L.polyline([[a.lat, a.lon], [b.lat, b.lon]], { color: ok ? "#1e9e5a" : "#d33", weight: 4, dashArray: ok ? null : "8 8" }).addTo(linkLayer);
  for (const e of [a, b]) L.circleMarker([e.lat, e.lon], { radius: 9, color: "#ff2d95", weight: 3, fill: false }).addTo(linkLayer);
  drawProfile(prof, stepM, p, hA, hB);
  status("");
}
function drawProfile(prof, stepM, p, hA, hB) {
  const cv = $("profile"), ctx = cv.getContext("2d"), W = cv.width, H = cv.height, n = prof.length - 1;
  const hs = [prof[0] + hA, prof[n] + hB], lo = Math.min(...prof) - 20, hi = Math.max(...prof, ...hs) + 20;
  const X = i => (i / n) * W, Y = h => H - ((h - lo) / (hi - lo)) * H;
  ctx.clearRect(0, 0, W, H);
  ctx.beginPath(); ctx.moveTo(0, H);
  for (let i = 0; i <= n; i++) ctx.lineTo(X(i), Y(prof[i]));
  ctx.lineTo(W, H); ctx.fillStyle = "#8a7a5c"; ctx.fill();
  ctx.beginPath(); ctx.moveTo(0, Y(hs[0])); ctx.lineTo(W, Y(hs[1]));
  ctx.strokeStyle = "#ff2d95"; ctx.lineWidth = 2; ctx.stroke();
  ctx.fillStyle = "#888"; ctx.font = "11px system-ui";
  ctx.fillText(`${Math.round(Math.max(...prof))} m`, 4, 12);
}

// ── Wiring ───────────────────────────────────────────────────────────────────
$("freq").onchange = () => { const d = PRESETS[$("freq").value]; $("eirp").value = d.eirp; $("sens").value = d.sens; legend(Number($("freq").value)); };
$("zones").onclick = () => computeZones(visibleNodes(), params());
$("clear").onclick = () => { if (zoneLayer) map.removeLayer(zoneLayer); zoneLayer = grid = null; $("zonestats").textContent = ""; status(""); };
$("link").onclick = () => setLinking(!linking);
$("add").onclick = () => { adding = !adding; $("add").classList.toggle("on", adding); map.getContainer().style.cursor = adding ? "crosshair" : ""; status(adding ? "Click the map where your Skein node is." : ""); };
$("export").onclick = () => { const t = JSON.stringify(localSkein(), null, 1); navigator.clipboard?.writeText(t); $("exported").value = t; status("Copied. Send it as a pull request to site/data/skein-nodes.json."); };
$("routers").onchange = drawNodes;
for (const [id, key] of [["l-skein", "skein"], ["l-mesh", "mesh"], ["l-masts", "masts"]]) {
  $(id).onchange = async () => {
    if (!$(id).checked) return map.removeLayer(layers[key]);
    if (key === "masts" && !masts) {
      masts = await fetch("data/emitters.json").then(r => r.json());
      masts.forEach(e => L.circleMarker([e[0], e[1]], { radius: 2.5, color: "#777", weight: 0, fillOpacity: 0.6, interactive: false }).addTo(layers.masts));
    }
    layers[key].addTo(map);
  };
}
legend(868);

Promise.all([fetch("data/meshtastic.json").then(r => r.json()), fetch("data/skein-nodes.json").then(r => r.json()).catch(() => [])]).then(([mesh, skein]) => {
  meshNodes = mesh.map(([lat, lon, role, hw]) => ({ lat, lon, role, hw, kind: "mesh" }));
  skeinNodes = [...skein, ...localSkein()].map(n => ({ ...n, kind: "skein", role: n.role || "SKEIN" }));
  drawNodes();
});

// Shareable view: ?view=lat,lon,zoom&zones=1 opens there (and computes the zones of the nodes in view).
const q = new URLSearchParams(location.search);
if (q.get("link")) {
  const [a, b, c, d] = q.get("link").split(",").map(Number);
  map.fitBounds([[a, b], [c, d]]);
  setTimeout(() => checkLink({ lat: a, lon: b, kind: "point" }, { lat: c, lon: d, kind: "point" }, params()), 800);
}
if (q.get("view")) {
  const [la, lo, z] = q.get("view").split(",").map(Number);
  map.setView([la, lo], z || 9);
  if (q.get("zones")) map.once("moveend", () => setTimeout(() => $("zones").click(), 800));
}
