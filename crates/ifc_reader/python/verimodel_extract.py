#!/usr/bin/env python3
"""VeriModel IFC extractor.

Reads one IFC file with IfcOpenShell and streams FACTS to stdout as NDJSON
(one JSON object per line). It never decides pass/fail: the Rust core does.

Line kinds, in order:
  header    file/schema/units/application facts
  spatial   the spatial structure tree (site > building > storey > space ...)
  element   one line per IfcProduct (properties, quantities, classification...)
  geometry  one line per shape the geometry kernel could build (bbox, volume, area)
  ids       IDS audit results (only when --ids is given)
  warning   non-fatal problem, plain language
  end       element_count and a summary
  error     fatal problem {code, message, hint}; the process then exits 1

All numeric quantities are converted to SI base units (m, m2, m3, kg).
Geometry is emitted in metres (IfcOpenShell converts world coordinates to SI).

Usage: verimodel_extract.py --ifc model.ifc [--ids spec.ids] [--no-geometry]
                            [--threads N] [--mode full|inspect] [--check-env]
"""
from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import sys
import tempfile
import traceback
import zipfile

PROTOCOL_VERSION = 1


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj, ensure_ascii=False, allow_nan=False, separators=(",", ":")))
    sys.stdout.write("\n")


def fail(code: str, message: str, hint: str) -> None:
    emit({"kind": "error", "code": code, "message": message, "hint": hint})
    sys.stdout.flush()
    sys.exit(1)


def safe_float(v):
    try:
        x = float(v)
    except (TypeError, ValueError):
        return None
    if x != x or x in (float("inf"), float("-inf")):
        return None
    return x


# --------------------------------------------------------------------------
# environment
# --------------------------------------------------------------------------

def check_env() -> dict:
    info = {"python": sys.version.split()[0], "ifcopenshell_version": None, "ifctester": False}
    try:
        import ifcopenshell  # noqa: F401

        info["ifcopenshell_version"] = getattr(ifcopenshell, "version", "unknown")
    except Exception as e:  # pragma: no cover - environment dependent
        return {**info, "error": str(e)}
    try:
        import ifctester  # noqa: F401

        info["ifctester"] = True
    except Exception:
        info["ifctester"] = False
    return info


# --------------------------------------------------------------------------
# value conversion
# --------------------------------------------------------------------------

def to_json_value(v):
    """Convert an IfcOpenShell property value into JSON-safe data."""
    if v is None:
        return None
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        f = safe_float(v)
        return f
    if isinstance(v, str):
        return v
    if isinstance(v, (list, tuple, set)):
        return [to_json_value(x) for x in v]
    if isinstance(v, dict):
        # complex / nested property: flatten to "a.b" keys is done by caller; keep as text here
        return {str(k): to_json_value(x) for k, x in v.items() if k != "id"}
    # IfcOpenShell entity instance or enum
    try:
        if hasattr(v, "wrappedValue"):
            return to_json_value(v.wrappedValue)
    except Exception:
        pass
    return str(v)


def flatten_props(props: dict) -> dict:
    """Flatten nested complex properties into dotted keys, values JSON safe."""
    out = {}
    for k, v in props.items():
        if k == "id":
            continue
        jv = to_json_value(v)
        if isinstance(jv, dict):
            for kk, vv in jv.items():
                if isinstance(vv, dict):
                    out[f"{k}.{kk}"] = json.dumps(vv, ensure_ascii=False)
                else:
                    out[f"{k}.{kk}"] = vv
        else:
            out[k] = jv
    return out


# --------------------------------------------------------------------------
# units
# --------------------------------------------------------------------------

def unit_facts(f):
    import ifcopenshell.util.unit as uu

    def scale(unit_type, fallback):
        try:
            return float(uu.calculate_unit_scale(f, unit_type))
        except Exception:
            return fallback

    def name(unit_type):
        try:
            u = uu.get_project_unit(f, unit_type)
            if u is None:
                return None
            try:
                return uu.get_full_unit_name(u)
            except Exception:
                return getattr(u, "Name", None)
        except Exception:
            return None

    length_to_m = scale("LENGTHUNIT", 1.0)
    area_to_m2 = scale("AREAUNIT", length_to_m ** 2)
    volume_to_m3 = scale("VOLUMEUNIT", length_to_m ** 3)
    mass_to_kg = scale("MASSUNIT", 1.0)
    length_name = name("LENGTHUNIT")
    imperial = False
    try:
        u = uu.get_project_unit(f, "LENGTHUNIT")
        if u is not None and u.is_a("IfcConversionBasedUnit"):
            imperial = (u.Name or "").upper() in ("INCH", "FOOT", "YARD", "MILE")
    except Exception:
        pass
    return {
        "length_unit": length_name,
        "length_to_m": length_to_m,
        "area_unit": name("AREAUNIT"),
        "area_to_m2": area_to_m2,
        "volume_unit": name("VOLUMEUNIT"),
        "volume_to_m3": volume_to_m3,
        "system": "imperial" if imperial else "metric",
    }, mass_to_kg


# --------------------------------------------------------------------------
# schema helpers
# --------------------------------------------------------------------------

class SchemaInfo:
    def __init__(self, f):
        import ifcopenshell

        self.schema_name = f.schema
        self.schema = ifcopenshell.ifcopenshell_wrapper.schema_by_name(f.schema_identifier if hasattr(f, "schema_identifier") else f.schema)
        self._cache = {}

    def ancestors(self, type_name: str):
        if type_name in self._cache:
            return self._cache[type_name]
        chain = []
        try:
            d = self.schema.declaration_by_name(type_name)
            while d is not None:
                chain.append(d.name())
                d = d.supertype()
        except Exception:
            chain = [type_name]
        self._cache[type_name] = chain
        return chain


def normalise_schema(name: str) -> str:
    up = (name or "").upper().replace(".", "X")
    base = up.split("_")[0]
    if base in ("IFC4X0", "IFC4X1", "IFC4X2"):
        return "IFC4"
    return base


# --------------------------------------------------------------------------
# spatial structure
# --------------------------------------------------------------------------

def is_spatial(e, schema: SchemaInfo) -> bool:
    anc = schema.ancestors(e.is_a())
    return "IfcSpatialElement" in anc or "IfcSpatialStructureElement" in anc


def spatial_tree(f, schema: SchemaInfo, units):
    import ifcopenshell.util.element as ue

    def node(e):
        elev = None
        if e.is_a("IfcBuildingStorey"):
            elev = safe_float(getattr(e, "Elevation", None))
            if elev is not None:
                elev = elev * units["length_to_m"]
        kids = []
        try:
            parts = ue.get_parts(e) or []
        except Exception:
            parts = []
        for p in parts:
            if is_spatial(p, schema):
                kids.append(node(p))
        kids.sort(key=lambda n: ((n["elevation_m"] if n["elevation_m"] is not None else 0.0), n["name"] or "", n["global_id"]))
        return {
            "global_id": e.GlobalId,
            "ifc_type": e.is_a(),
            "name": e.Name,
            "elevation_m": elev,
            "children": kids,
        }

    roots = []
    for proj in f.by_type("IfcProject"):
        try:
            parts = ue.get_parts(proj) or []
        except Exception:
            parts = []
        for p in parts:
            if is_spatial(p, schema):
                roots.append(node(p))
    if not roots:
        # Orphan spatial elements (no project aggregation): list them flat so checks can still see them.
        for e in f.by_type("IfcSpatialStructureElement") if schema.schema_name.startswith("IFC2X3") else f.by_type("IfcSpatialElement"):
            try:
                if ue.get_aggregate(e) is None:
                    roots.append(node(e))
            except Exception:
                roots.append(node(e))
    roots.sort(key=lambda n: (n["name"] or "", n["global_id"]))
    return roots


def storey_of(e, schema: SchemaInfo):
    """Walk containment / aggregation upwards until an IfcBuildingStorey is found."""
    import ifcopenshell.util.element as ue

    cur = e
    for _ in range(32):
        try:
            parent = ue.get_container(cur) if not is_spatial(cur, schema) else ue.get_aggregate(cur)
        except Exception:
            parent = None
        if parent is None:
            try:
                parent = ue.get_aggregate(cur)
            except Exception:
                parent = None
        if parent is None:
            return None
        if parent.is_a("IfcBuildingStorey"):
            return parent.Name
        cur = parent
    return None


# --------------------------------------------------------------------------
# element facts
# --------------------------------------------------------------------------

QUANTITY_VALUE_INDEX = {
    "IfcQuantityLength": ("LengthValue", "length"),
    "IfcQuantityArea": ("AreaValue", "area"),
    "IfcQuantityVolume": ("VolumeValue", "volume"),
    "IfcQuantityCount": ("CountValue", "count"),
    "IfcQuantityWeight": ("WeightValue", "mass"),
    "IfcQuantityTime": ("TimeValue", "time"),
    "IfcQuantityNumber": ("NumberValue", "count"),
}


def quantities_of(e, units, mass_to_kg):
    """Quantity sets with values converted to SI, walking IfcElementQuantity directly so
    each quantity's kind (length/area/volume) is known for conversion."""
    import ifcopenshell.util.element as ue

    out = {}
    try:
        qsets = ue.get_psets(e, qtos_only=True)
    except Exception:
        qsets = {}
    # Map set name -> entity so we can inspect quantity kinds
    for qname, qvals in qsets.items():
        qid = qvals.get("id")
        qset = None
        if qid is not None:
            try:
                qset = e.file.by_id(qid)
            except Exception:
                qset = None
        values = {}
        if qset is not None and qset.is_a("IfcElementQuantity"):
            for q in qset.Quantities or []:
                kind = QUANTITY_VALUE_INDEX.get(q.is_a())
                if kind is None:
                    continue
                attr, dim = kind
                raw = safe_float(getattr(q, attr, None))
                if raw is None:
                    continue
                if dim == "length":
                    raw *= units["length_to_m"]
                elif dim == "area":
                    raw *= units["area_to_m2"]
                elif dim == "volume":
                    raw *= units["volume_to_m3"]
                elif dim == "mass":
                    raw *= mass_to_kg
                values[q.Name] = raw
        else:
            for k, v in qvals.items():
                if k == "id":
                    continue
                fv = safe_float(v)
                if fv is not None:
                    values[k] = fv
        if values:
            out[qname] = values
    return out


def classifications_of(e):
    import ifcopenshell.util.classification as uc

    out = []
    try:
        refs = uc.get_references(e)
    except Exception:
        refs = []
    for r in refs:
        system = None
        try:
            c = uc.get_classification(r)
            system = getattr(c, "Name", None) if c is not None else None
        except Exception:
            system = None
        code = getattr(r, "Identification", None) if hasattr(r, "Identification") else getattr(r, "ItemReference", None)
        out.append({"system": system, "code": code, "name": getattr(r, "Name", None)})
    out.sort(key=lambda c: (c["system"] or "", c["code"] or "", c["name"] or ""))
    return out


def element_facts(e, schema: SchemaInfo, units, mass_to_kg):
    import ifcopenshell.util.element as ue

    psets = {}
    try:
        raw = ue.get_psets(e, psets_only=True)
    except Exception as ex:
        raw = {}
    for name, props in raw.items():
        psets[name] = flatten_props(props)
    container = None
    try:
        c = ue.get_container(e) if not is_spatial(e, schema) else ue.get_aggregate(e)
    except Exception:
        c = None
    if c is not None:
        container = {"global_id": c.GlobalId, "ifc_type": c.is_a(), "name": c.Name}
    materials = []
    try:
        for m in ue.get_materials(e) or []:
            n = getattr(m, "Name", None)
            if n:
                materials.append(n)
    except Exception:
        pass
    materials = sorted(set(materials))
    type_name = None
    try:
        t = ue.get_type(e)
        type_name = getattr(t, "Name", None) if t is not None else None
    except Exception:
        pass
    predefined = None
    try:
        predefined = ue.get_predefined_type(e)
    except Exception:
        pass
    return {
        "kind": "element",
        "global_id": e.GlobalId if e.GlobalId is not None else "",
        "step_id": e.id(),
        "ifc_type": e.is_a(),
        "ancestors": schema.ancestors(e.is_a()),
        "predefined_type": predefined,
        "name": e.Name,
        "description": getattr(e, "Description", None),
        "object_type": getattr(e, "ObjectType", None),
        "tag": getattr(e, "Tag", None),
        "type_name": type_name,
        "storey": storey_of(e, schema),
        "container": container,
        "psets": psets,
        "quantities": quantities_of(e, units, mass_to_kg),
        "classifications": classifications_of(e),
        "materials": materials,
        "has_representation": getattr(e, "Representation", None) is not None,
    }


# --------------------------------------------------------------------------
# geometry
# --------------------------------------------------------------------------

def stream_geometry(f, threads: int):
    import ifcopenshell.geom
    import ifcopenshell.util.shape as us

    settings = ifcopenshell.geom.settings()
    settings.set("use-world-coords", True)
    try:
        settings.set("weld-vertices", True)
    except Exception:
        pass
    it = ifcopenshell.geom.iterator(settings, f, max(1, threads))
    if not it.initialize():
        return 0
    n = 0
    while True:
        try:
            shape = it.get()
            g = shape.geometry
            verts = g.verts
            faces = g.faces
            tri = len(faces) // 3
            if len(verts) >= 3:
                xs, ys, zs = verts[0::3], verts[1::3], verts[2::3]
                bmin = [min(xs), min(ys), min(zs)]
                bmax = [max(xs), max(ys), max(zs)]
            else:
                bmin = [0.0, 0.0, 0.0]
                bmax = [0.0, 0.0, 0.0]
            vol = None
            area = None
            if tri > 0:
                try:
                    vol = safe_float(abs(us.get_volume(g)))
                except Exception:
                    vol = None
                try:
                    area = safe_float(us.get_area(g))
                except Exception:
                    area = None
            emit({
                "kind": "geometry",
                "global_id": shape.guid,
                "step_id": int(shape.id),
                "bbox_min": bmin,
                "bbox_max": bmax,
                "volume_m3": vol,
                "area_m2": area,
                "triangle_count": tri,
            })
            n += 1
        except Exception as ex:
            emit({"kind": "warning", "message": f"geometry: {ex}"})
        if not it.next():
            break
    return n


# --------------------------------------------------------------------------
# IDS
# --------------------------------------------------------------------------

def run_ids(f, ids_path: str) -> dict:
    import ifctester.ids

    spec = ifctester.ids.open(ids_path)
    spec.validate(f)
    specs_out = []
    for s in spec.specifications:
        max_occ = getattr(s, "maxOccurs", None)
        min_occ = getattr(s, "minOccurs", None)
        if max_occ == 0:
            cardinality = "prohibited"
        elif min_occ in (0, "0"):
            cardinality = "optional"
        else:
            cardinality = "required"
        applicability = []
        for a in s.applicability:
            try:
                applicability.append(a.to_string("applicability"))
            except Exception:
                applicability.append(type(a).__name__)
        reqs = []
        for r in s.requirements:
            try:
                desc = r.to_string("requirement", s, r)
            except Exception:
                desc = type(r).__name__
            failed = []
            for fl in getattr(r, "failures", []) or []:
                el = fl.get("element")
                failed.append({
                    "global_id": getattr(el, "GlobalId", "") or "",
                    "ifc_type": el.is_a() if el is not None else "",
                    "name": getattr(el, "Name", None),
                    "reason": str(fl.get("reason", "")),
                })
            failed.sort(key=lambda x: (x["global_id"], x["reason"]))
            reqs.append({
                "description": desc,
                "facet": type(r).__name__.lower(),
                "passed": bool(r.status) if r.status is not None else True,
                "failed_entities": failed,
            })
        specs_out.append({
            "name": s.name or "",
            "description": s.description,
            "instructions": s.instructions,
            "applicability": applicability,
            "cardinality": cardinality,
            "applicable_count": len(getattr(s, "applicable_entities", []) or []),
            "passed": bool(s.status) if s.status is not None else True,
            "requirements": reqs,
        })
    return {"kind": "ids", "ids_file": os.path.basename(ids_path), "title": (spec.info or {}).get("title"), "specifications": specs_out}


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def open_ifc(path: str):
    import ifcopenshell

    if path.lower().endswith(".ifczip") or zipfile.is_zipfile(path):
        with zipfile.ZipFile(path) as z:
            names = [n for n in z.namelist() if n.lower().endswith(".ifc")]
            if not names:
                fail("ifczip_empty", "The .ifczip archive does not contain an .ifc file.", "Check the archive content; it must contain exactly one .ifc file.")
            tmpdir = tempfile.mkdtemp(prefix="verimodel_")
            extracted = z.extract(names[0], tmpdir)
            return ifcopenshell.open(extracted)
    return ifcopenshell.open(path)


def main() -> int:
    ap = argparse.ArgumentParser(description="VeriModel IFC extractor (IfcOpenShell bridge)")
    ap.add_argument("--ifc")
    ap.add_argument("--ids")
    ap.add_argument("--no-geometry", action="store_true")
    ap.add_argument("--threads", type=int, default=0)
    ap.add_argument("--mode", choices=["full", "inspect"], default="full")
    ap.add_argument("--check-env", action="store_true")
    args = ap.parse_args()

    if args.check_env:
        emit({"kind": "env", "protocol": PROTOCOL_VERSION, **check_env()})
        return 0
    if not args.ifc:
        fail("no_input", "No IFC file given.", "Pass --ifc <file>.")
    if not os.path.isfile(args.ifc):
        fail("not_found", f"The file '{args.ifc}' does not exist.", "Check the path and that the file was not moved.")

    try:
        import ifcopenshell  # noqa: F401
    except Exception as ex:
        fail("ifcopenshell_missing", f"IfcOpenShell is not available in this Python ({sys.executable}): {ex}", "Install it with: pip install ifcopenshell ifctester")

    try:
        f = open_ifc(args.ifc)
    except Exception as ex:
        fail("parse_failed", f"The file could not be parsed as IFC: {ex}", "Check that the file is a STEP (.ifc) file, is not truncated, and was exported completely.")

    try:
        schema = SchemaInfo(f)
    except Exception as ex:
        fail("schema_unknown", f"The IFC schema of the file is not supported: {ex}", "VeriModel supports IFC2X3, IFC4 and IFC4X3.")

    units, mass_to_kg = unit_facts(f)
    h = f.header
    try:
        mvd = "; ".join(h.file_description.description or []) or None
    except Exception:
        mvd = None
    try:
        app = h.file_name.originating_system or h.file_name.preprocessor_version or None
    except Exception:
        app = None
    projects = f.by_type("IfcProject")
    geometry_available = not args.no_geometry and args.mode == "full"
    if geometry_available:
        try:
            import ifcopenshell.geom  # noqa: F401
        except Exception:
            geometry_available = False
            emit({"kind": "warning", "message": "The geometry kernel of IfcOpenShell is not available; geometry checks will be skipped."})

    emit({
        "kind": "header",
        "protocol": PROTOCOL_VERSION,
        "schema": normalise_schema(f.schema),
        "schema_raw": f.schema,
        "mvd": mvd,
        "authoring_tool": app,
        "project_name": projects[0].Name if projects else None,
        "units": units,
        "file_size_bytes": os.path.getsize(args.ifc),
        "ifcopenshell_version": getattr(sys.modules["ifcopenshell"], "version", "unknown"),
        "geometry_available": geometry_available,
    })
    emit({"kind": "spatial", "tree": spatial_tree(f, schema, units)})

    count = 0
    products = f.by_type("IfcProduct")
    for e in products:
        try:
            emit(element_facts(e, schema, units, mass_to_kg))
            count += 1
        except Exception as ex:
            emit({"kind": "warning", "message": f"element #{e.id()} ({e.is_a()}) could not be read: {ex}"})
    sys.stdout.flush()

    if args.mode == "full":
        if geometry_available:
            try:
                threads = args.threads or multiprocessing.cpu_count()
                stream_geometry(f, threads)
            except Exception as ex:
                emit({"kind": "warning", "message": f"The geometry kernel failed: {ex}. Geometry checks will be skipped."})
                emit({"kind": "geometry_unavailable"})
        if args.ids:
            if not os.path.isfile(args.ids):
                fail("ids_not_found", f"The IDS file '{args.ids}' does not exist.", "Check the path of the IDS file.")
            try:
                emit(run_ids(f, args.ids))
            except ImportError as ex:
                fail("ifctester_missing", f"The IDS engine (ifctester) is not installed: {ex}", "Install it with: pip install ifctester")
            except Exception as ex:
                fail("ids_failed", f"The IDS file could not be processed: {ex}", "Check that the IDS file follows the buildingSMART IDS 1.0 schema.")

    emit({"kind": "end", "element_count": count})
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception as ex:  # last resort: never a bare traceback on stdout
        sys.stderr.write(traceback.format_exc())
        fail("internal", f"Unexpected reader failure: {ex}", "See the log for details and report this as a bug.")
