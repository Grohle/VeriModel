#!/usr/bin/env python3
"""Generate `broken_building.ifc`: a small IFC4 model with deliberate defects.

Run:  python3 tests/fixtures/make_broken.py
The file is committed; this script documents exactly how it was produced so
anyone can regenerate it. Defects (what VeriModel must catch):

  1. duplicate GlobalId        two walls share the same GlobalId          -> ID-001
  2. invalid GlobalId          a slab with a 7-character GlobalId         -> ID-002
  3. element without storey    a column not contained in any storey      -> SP-001
  4. zero-volume wall          a wall extruded with zero depth            -> GEO-002 / QT-001
  5. far from origin           a beam placed 25 km from the origin        -> SP-004
  6. missing FireRating        no wall carries Pset_WallCommon.FireRating -> IDS / CFG-PSET
  7. empty property value      Pset_WallCommon.Reference = ""             -> PR-001
  8. duplicate names           two walls named "Wall"                     -> ID-003

Units are MILLIMETRES on purpose, so unit conversion is exercised: the good
wall is 5000 mm long, 3000 mm high and 200 mm thick (= 3.0 m3). The
ifcopenshell.api geometry helpers take METRES and convert to project units.
"""
import os
import sys

import numpy as np
import ifcopenshell
import ifcopenshell.api
import ifcopenshell.api.root
import ifcopenshell.api.unit
import ifcopenshell.api.context
import ifcopenshell.api.project
import ifcopenshell.api.aggregate
import ifcopenshell.api.spatial
import ifcopenshell.api.geometry
import ifcopenshell.api.pset
import ifcopenshell.api.material
import ifcopenshell.api.classification

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "broken_building.ifc")


def main() -> None:
    f = ifcopenshell.api.project.create_file(version="IFC4")
    project = ifcopenshell.api.root.create_entity(f, ifc_class="IfcProject", name="VeriModel broken sample")
    mm = ifcopenshell.api.unit.add_si_unit(f, unit_type="LENGTHUNIT", prefix="MILLI")
    area = ifcopenshell.api.unit.add_si_unit(f, unit_type="AREAUNIT")
    vol = ifcopenshell.api.unit.add_si_unit(f, unit_type="VOLUMEUNIT")
    ifcopenshell.api.unit.assign_unit(f, units=[mm, area, vol])
    model_ctx = ifcopenshell.api.context.add_context(f, context_type="Model")
    body = ifcopenshell.api.context.add_context(
        f, context_type="Model", context_identifier="Body", target_view="MODEL_VIEW", parent=model_ctx
    )

    site = ifcopenshell.api.root.create_entity(f, ifc_class="IfcSite", name="Site")
    building = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuilding", name="Broken House")
    storey = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuildingStorey", name="Level 1")
    storey.Elevation = 0.0
    ifcopenshell.api.aggregate.assign_object(f, relating_object=project, products=[site])
    ifcopenshell.api.aggregate.assign_object(f, relating_object=site, products=[building])
    ifcopenshell.api.aggregate.assign_object(f, relating_object=building, products=[storey])

    concrete = ifcopenshell.api.material.add_material(f, name="Concrete C30/37")

    def box(element, length_mm, height_mm, thickness_mm, x=0.0, y=0.0, z=0.0):
        # The API takes metres and writes project units (mm) into the file.
        rep = ifcopenshell.api.geometry.add_wall_representation(
            f, context=body, length=length_mm / 1000.0, height=height_mm / 1000.0, thickness=thickness_mm / 1000.0
        )
        ifcopenshell.api.geometry.assign_representation(f, product=element, representation=rep)
        matrix = np.eye(4)
        matrix[:, 3][0:3] = (x / 1000.0, y / 1000.0, z / 1000.0)
        ifcopenshell.api.geometry.edit_object_placement(f, product=element, matrix=matrix, is_si=True)

    def wall(name, **kw):
        w = ifcopenshell.api.root.create_entity(f, ifc_class="IfcWall", name=name)
        box(w, **kw)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[w])
        ifcopenshell.api.material.assign_material(f, products=[w], material=concrete)
        return w

    # 1 + 8: two walls, same GlobalId, same name
    w1 = wall("Wall", length_mm=5000.0, height_mm=3000.0, thickness_mm=200.0)
    w2 = wall("Wall", length_mm=4000.0, height_mm=3000.0, thickness_mm=200.0, y=3000.0)
    w2.GlobalId = w1.GlobalId
    # 6 + 7: psets without FireRating, with an empty Reference
    for w in (w1, w2):
        pset = ifcopenshell.api.pset.add_pset(f, product=w, name="Pset_WallCommon")
        ifcopenshell.api.pset.edit_pset(f, pset=pset, properties={"IsExternal": True, "Reference": ""})

    # 4: zero-volume wall (zero thickness)
    w3 = wall("Wall-Zero", length_mm=3000.0, height_mm=3000.0, thickness_mm=0.0, y=6000.0)
    pset = ifcopenshell.api.pset.add_pset(f, product=w3, name="Pset_WallCommon")
    ifcopenshell.api.pset.edit_pset(f, pset=pset, properties={"IsExternal": False, "FireRating": "REI60"})

    # 2: invalid GlobalId on a slab
    slab = ifcopenshell.api.root.create_entity(f, ifc_class="IfcSlab", name="Slab-1")
    box(slab, length_mm=6000.0, height_mm=250.0, thickness_mm=6000.0, z=-250.0)
    ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[slab])
    slab.GlobalId = "BADID01"

    # 3: column with no spatial container
    col = ifcopenshell.api.root.create_entity(f, ifc_class="IfcColumn", name="Column-Orphan")
    box(col, length_mm=300.0, height_mm=3000.0, thickness_mm=300.0, x=8000.0)

    # 5: beam 25 km away
    beam = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBeam", name="Beam-Far")
    box(beam, length_mm=4000.0, height_mm=400.0, thickness_mm=200.0, x=25_000_000.0, y=25_000_000.0, z=3000.0)
    ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[beam])

    # a healthy door so at least one element passes everything
    door = ifcopenshell.api.root.create_entity(f, ifc_class="IfcDoor", name="Door-1")
    box(door, length_mm=900.0, height_mm=2100.0, thickness_mm=100.0, x=1000.0)
    ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[door])
    pset = ifcopenshell.api.pset.add_pset(f, product=door, name="Pset_DoorCommon")
    ifcopenshell.api.pset.edit_pset(f, pset=pset, properties={"FireRating": "EI30", "IsExternal": False})

    f.write(OUT)
    print(f"wrote {OUT} ({os.path.getsize(OUT)} bytes, {len(f.by_type('IfcProduct'))} products)")


if __name__ == "__main__":
    sys.exit(main())
