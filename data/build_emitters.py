"""Build site/data/emitters.json from the ANFR open data "Données sur les réseaux mobiles" CSV.

Usage: python build_emitters.py [anfr_raw.csv]
Download the CSV from https://data.anfr.fr (dataset observatoire_2g_3g_4g), Licence Ouverte v2.0.
One entry per support (mast): [lat, lon, height_m, "operators", "bands (MHz)"]. Only operational stations.
"""
import csv, json, re, sys
from pathlib import Path

src = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).with_name("anfr_raw.csv"))
sup = {}
with open(src, encoding="utf-8-sig", newline="") as f:
    for r in csv.DictReader(f, delimiter=";"):
        if "opérationnel" not in r["statut"]:
            continue
        try:
            lat, lon = (float(x) for x in r["coordonnees"].split(","))
        except ValueError:
            continue
        s = sup.setdefault(r["sup_id"], {"lat": lat, "lon": lon, "h": 0.0, "ops": set(), "f": set()})
        try:
            s["h"] = max(s["h"], float(r["sup_nm_haut"].replace(",", ".") or 0))
        except ValueError:
            pass
        s["ops"].add(r["adm_lb_nom"].replace(" TELECOM", "").replace(" MOBILE", "").title())
        m = re.search(r"(\d{3,4})$", r["emr_lb_systeme"])
        if m:
            s["f"].add(int(m.group(1)))
out = [[round(s["lat"], 5), round(s["lon"], 5), round(s["h"]), ",".join(sorted(s["ops"])), ",".join(map(str, sorted(s["f"])))] for s in sup.values()]
Path(__file__).with_name("emitters.json").write_text(json.dumps(out, separators=(",", ":")), encoding="utf-8")
print(len(out), "supports")
