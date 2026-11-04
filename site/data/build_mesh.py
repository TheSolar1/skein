"""Build site/data/meshtastic.json: a France snapshot of the public Meshtastic map (https://meshtastic.liamcottle.net).

Usage: python build_mesh.py
Only position, role and hardware are kept (no names, no ids). Positions are the ones each owner chose to publish
(many are deliberately blurred). One entry per node: [lat, lon, role, hardware].
"""
import json, urllib.request
from pathlib import Path

req = urllib.request.Request("https://meshtastic.liamcottle.net/api/v1/nodes", headers={"User-Agent": "skein-map"})
nodes = json.load(urllib.request.urlopen(req, timeout=120))["nodes"]
out = []
for n in nodes:
    if not n.get("latitude") or not n.get("longitude"):
        continue
    lat, lon = n["latitude"] / 1e7, n["longitude"] / 1e7
    if 41.0 < lat < 51.5 and -5.5 < lon < 9.7:  # metropolitan France and Corsica
        out.append([round(lat, 4), round(lon, 4), n.get("role_name") or "CLIENT", n.get("hardware_model_name") or ""])
Path(__file__).with_name("meshtastic.json").write_text(json.dumps(out, separators=(",", ":")), encoding="utf-8")
print(len(out), "nodes")
