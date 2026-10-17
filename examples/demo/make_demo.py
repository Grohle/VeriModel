#!/usr/bin/env python3
"""Generate `examples/demo/demo_building.ifc`: the model VeriModel ships as its
always-available demo.

A small two-storey office pavilion written with the IfcOpenShell API (IFC4,
millimetres): walls, slabs, columns, beams, doors, windows, a roof, spaces, a
duct run, with property sets, base quantities, Uniclass classification and
materials. It is deliberately *almost* clean so a first run shows what a real
review looks like: a healthy score, three warnings and three real errors
(with examples/demo/project.yaml, where classification is required).

Deliberate defects:
  * ground-floor door D-003 has Pset_DoorCommon but no FireRating   -> IDS / CFG-PSET
  * column C-005 is not contained in any storey                     -> SP-001 (error)
  * window W-004 carries an empty Pset_WindowCommon.Reference       -> PR-001 (warning)
  * duct segment MEP-002 has no classification                      -> CL-001 (error: the demo config requires classification)
  * two walls on level 2 share the name "WL-201"                    -> ID-003 (warning)

Run:  python3 examples/demo/make_demo.py
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
import ifcopenshell.api.type

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "demo_building.ifc")


def main() -> None:
    f = ifcopenshell.api.project.create_file(version="IFC4")
    project = ifcopenshell.api.root.create_entity(f, ifc_class="IfcProject", name="VeriModel demo pavilion")
    mm = ifcopenshell.api.unit.add_si_unit(f, unit_type="LENGTHUNIT", prefix="MILLI")
    area = ifcopenshell.api.unit.add_si_unit(f, unit_type="AREAUNIT")
    vol = ifcopenshell.api.unit.add_si_unit(f, unit_type="VOLUMEUNIT")
    ifcopenshell.api.unit.assign_unit(f, units=[mm, area, vol])
    model_ctx = ifcopenshell.api.context.add_context(f, context_type="Model")
    body = ifcopenshell.api.context.add_context(f, context_type="Model", context_identifier="Body", target_view="MODEL_VIEW", parent=model_ctx)

    site = ifcopenshell.api.root.create_entity(f, ifc_class="IfcSite", name="Demo site")
    building = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuilding", name="Demo pavilion")
    l1 = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuildingStorey", name="Level 1")
    l1.Elevation = 0.0
    l2 = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuildingStorey", name="Level 2")
    l2.Elevation = 3500.0
    roof_lvl = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBuildingStorey", name="Roof")
    roof_lvl.Elevation = 7000.0
    ifcopenshell.api.aggregate.assign_object(f, relating_object=project, products=[site])
    ifcopenshell.api.aggregate.assign_object(f, relating_object=site, products=[building])
    ifcopenshell.api.aggregate.assign_object(f, relating_object=building, products=[l1, l2, roof_lvl])

    concrete = ifcopenshell.api.material.add_material(f, name="Concrete C30/37")
    brick = ifcopenshell.api.material.add_material(f, name="Brick masonry")
    steel = ifcopenshell.api.material.add_material(f, name="Steel S275")
    timber = ifcopenshell.api.material.add_material(f, name="Timber")
    glass = ifcopenshell.api.material.add_material(f, name="Glass")
    galv = ifcopenshell.api.material.add_material(f, name="Galvanised steel")

    system = ifcopenshell.api.classification.add_classification(f, classification="Uniclass 2015")

    def classify(products, code, name):
        ifcopenshell.api.classification.add_reference(f, products=products, identification=code, name=name, classification=system)

    def box(element, length_mm, height_mm, thickness_mm, x=0.0, y=0.0, z=0.0, rot_deg=0.0):
        rep = ifcopenshell.api.geometry.add_wall_representation(f, context=body, length=length_mm / 1000.0, height=height_mm / 1000.0, thickness=thickness_mm / 1000.0)
        ifcopenshell.api.geometry.assign_representation(f, product=element, representation=rep)
        m = np.eye(4)
        a = np.deg2rad(rot_deg)
        m[0:2, 0:2] = [[np.cos(a), -np.sin(a)], [np.sin(a), np.cos(a)]]
        m[0:3, 3] = (x / 1000.0, y / 1000.0, z / 1000.0)
        ifcopenshell.api.geometry.edit_object_placement(f, product=element, matrix=m, is_si=True)

    def add_qto(element, name, values):
        q = ifcopenshell.api.pset.add_qto(f, product=element, name=name)
        ifcopenshell.api.pset.edit_qto(f, qto=q, properties=values)

    def add_pset(element, name, values):
        p = ifcopenshell.api.pset.add_pset(f, product=element, name=name)
        ifcopenshell.api.pset.edit_pset(f, pset=p, properties=values)

    def wall(name, storey, length, x, y, z, rot=0.0, external=True, fire="REI90", thickness=250.0):
        w = ifcopenshell.api.root.create_entity(f, ifc_class="IfcWall", name=name, predefined_type="SOLIDWALL")
        box(w, length, 3500.0, thickness, x, y, z, rot)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[w])
        ifcopenshell.api.material.assign_material(f, products=[w], material=brick if external else concrete)
        add_pset(w, "Pset_WallCommon", {"IsExternal": external, "FireRating": fire, "LoadBearing": external, "Reference": name})
        add_qto(w, "Qto_WallBaseQuantities", {"Length": length, "Height": 3500.0, "Width": thickness, "GrossVolume": length * 3500.0 * thickness / 1e9, "NetVolume": length * 3500.0 * thickness / 1e9, "GrossSideArea": length * 3500.0 / 1e6, "NetSideArea": length * 3500.0 / 1e6})
        classify([w], "Ss_25_10_30" if external else "Ss_25_10_32", "External wall systems" if external else "Internal wall systems")
        return w

    def slab(name, storey, z, kind="FLOOR"):
        s = ifcopenshell.api.root.create_entity(f, ifc_class="IfcSlab", name=name, predefined_type=kind)
        box(s, 12000.0, 250.0, 8000.0, 0.0, 0.0, z - 250.0)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[s])
        ifcopenshell.api.material.assign_material(f, products=[s], material=concrete)
        add_pset(s, "Pset_SlabCommon", {"IsExternal": kind == "ROOF", "LoadBearing": True, "FireRating": "REI120", "Reference": name})
        add_qto(s, "Qto_SlabBaseQuantities", {"Depth": 250.0, "GrossArea": 96.0, "NetArea": 96.0, "GrossVolume": 24.0, "NetVolume": 24.0, "Perimeter": 40000.0})
        classify([s], "Ss_30_20_55", "Floor and roof slab systems" if kind != "ROOF" else "Roof structure systems")
        return s

    def column(name, storey, x, y, z, contained=True):
        c = ifcopenshell.api.root.create_entity(f, ifc_class="IfcColumn", name=name, predefined_type="COLUMN")
        box(c, 400.0, 3500.0, 400.0, x, y, z)
        if contained:
            ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[c])
        ifcopenshell.api.material.assign_material(f, products=[c], material=concrete)
        add_pset(c, "Pset_ColumnCommon", {"LoadBearing": True, "IsExternal": False, "FireRating": "R120", "Reference": name})
        add_qto(c, "Qto_ColumnBaseQuantities", {"Length": 3500.0, "CrossSectionArea": 0.16, "GrossVolume": 0.56, "NetVolume": 0.56})
        classify([c], "Ss_20_05_15", "Concrete column systems")
        return c

    def beam(name, storey, length, x, y, z, rot=0.0):
        b = ifcopenshell.api.root.create_entity(f, ifc_class="IfcBeam", name=name, predefined_type="BEAM")
        box(b, length, 500.0, 300.0, x, y, z, rot)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[b])
        ifcopenshell.api.material.assign_material(f, products=[b], material=steel)
        add_pset(b, "Pset_BeamCommon", {"LoadBearing": True, "IsExternal": False, "FireRating": "R60", "Reference": name})
        add_qto(b, "Qto_BeamBaseQuantities", {"Length": length, "CrossSectionArea": 0.15, "GrossVolume": length * 0.15 / 1000.0, "NetVolume": length * 0.15 / 1000.0})
        classify([b], "Ss_20_10_75", "Steel beam systems")
        return b

    def door(name, storey, x, y, z, fire="EI30", with_fire=True):
        d = ifcopenshell.api.root.create_entity(f, ifc_class="IfcDoor", name=name, predefined_type="DOOR")
        box(d, 1000.0, 2100.0, 100.0, x, y, z)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[d])
        ifcopenshell.api.material.assign_material(f, products=[d], material=timber)
        props = {"IsExternal": False, "Reference": name}
        if with_fire:
            props["FireRating"] = fire
        add_pset(d, "Pset_DoorCommon", props)
        add_qto(d, "Qto_DoorBaseQuantities", {"Width": 1000.0, "Height": 2100.0, "Area": 2.1})
        classify([d], "Pr_30_59_24", "Doorsets")
        return d

    def window(name, storey, x, y, z, reference=None):
        w = ifcopenshell.api.root.create_entity(f, ifc_class="IfcWindow", name=name, predefined_type="WINDOW")
        box(w, 1500.0, 1200.0, 80.0, x, y, z)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[w])
        ifcopenshell.api.material.assign_material(f, products=[w], material=glass)
        add_pset(w, "Pset_WindowCommon", {"IsExternal": True, "Reference": name if reference is None else reference, "ThermalTransmittance": 1.1})
        add_qto(w, "Qto_WindowBaseQuantities", {"Width": 1500.0, "Height": 1200.0, "Area": 1.8})
        classify([w], "Pr_30_59_98", "Windows")
        return w

    def space(name, storey, x, y, z, lx, ly):
        s = ifcopenshell.api.root.create_entity(f, ifc_class="IfcSpace", name=name, predefined_type="INTERNAL")
        box(s, lx, 3250.0, ly, x, y, z)
        ifcopenshell.api.aggregate.assign_object(f, relating_object=storey, products=[s])
        add_pset(s, "Pset_SpaceCommon", {"Reference": name, "IsExternal": False, "GrossPlannedArea": lx * ly / 1e6})
        add_qto(s, "Qto_SpaceBaseQuantities", {"Height": 3250.0, "GrossFloorArea": lx * ly / 1e6, "NetFloorArea": lx * ly / 1e6 * 0.95, "GrossVolume": lx * ly * 3250.0 / 1e9})
        classify([s], "SL_20_15_59", "Office spaces")
        return s

    def duct(name, storey, length, x, y, z, rot=0.0, classified=True):
        d = ifcopenshell.api.root.create_entity(f, ifc_class="IfcDuctSegment", name=name, predefined_type="RIGIDSEGMENT")
        box(d, length, 300.0, 300.0, x, y, z, rot)
        ifcopenshell.api.spatial.assign_container(f, relating_structure=storey, products=[d])
        ifcopenshell.api.material.assign_material(f, products=[d], material=galv)
        add_pset(d, "Pset_DuctSegmentTypeCommon", {"Reference": name, "NominalDiameterOrWidth": 300.0})
        add_qto(d, "Qto_DuctSegmentBaseQuantities", {"Length": length, "GrossCrossSectionArea": 0.09, "OuterSurfaceArea": length * 1.2 / 1000.0})
        if classified:
            classify([d], "Ss_65_40_33", "Ductwork systems")
        return d

    # ---- Level 1 -----------------------------------------------------------
    slab("SL-101", l1, 0.0)
    wall("WL-101", l1, 12000.0, 0, 0, 0)
    wall("WL-102", l1, 8000.0, 12000, 0, 0, 90)
    wall("WL-103", l1, 12000.0, 12000, 8000, 0, 180)
    wall("WL-104", l1, 8000.0, 0, 8000, 0, 270)
    wall("WL-105", l1, 6000.0, 6000, 250, 0, 90, external=False, fire="REI60", thickness=150.0)
    for i, (x, y) in enumerate([(300, 300), (6000, 300), (11300, 300), (300, 7300), (11300, 7300)]):
        column(f"C-10{i+1}", l1, x, y, 0)
    beam("B-101", l1, 11000.0, 500, 300, 3000)
    beam("B-102", l1, 11000.0, 500, 7300, 3000)
    door("D-101", l1, 2500, 0, 0)
    door("D-102", l1, 6000, 3000, 0, fire="EI60")
    door("D-103", l1, 9000, 0, 0, with_fire=False)          # defect: no FireRating
    window("W-101", l1, 1000, 8000, 1000)
    window("W-102", l1, 4000, 8000, 1000)
    window("W-103", l1, 8000, 8000, 1000)
    space("Office 1.01", l1, 250, 250, 0, 5600, 7500)
    space("Office 1.02", l1, 6150, 250, 0, 5600, 7500)
    duct("MEP-101", l1, 10000.0, 1000, 4000, 3100)
    duct("MEP-102", l1, 3000.0, 11000, 4000, 3100, 90, classified=False)   # defect: unclassified

    # ---- Level 2 -----------------------------------------------------------
    slab("SL-201", l2, 3500.0)
    wall("WL-201", l2, 12000.0, 0, 0, 3500)
    wall("WL-202", l2, 8000.0, 12000, 0, 3500, 90)
    wall("WL-201", l2, 12000.0, 12000, 8000, 3500, 180)     # defect: duplicate name
    wall("WL-204", l2, 8000.0, 0, 8000, 3500, 270)
    for i, (x, y) in enumerate([(300, 300), (6000, 300), (11300, 300), (300, 7300)]):
        column(f"C-20{i+1}", l2, x, y, 3500)
    column("C-205", l2, 11300, 7300, 3500, contained=False)  # defect: orphan column
    beam("B-201", l2, 11000.0, 500, 300, 6500)
    door("D-201", l2, 5500, 0, 3500)
    window("W-201", l2, 1000, 8000, 4500)
    window("W-202", l2, 4000, 8000, 4500)
    window("W-203", l2, 8000, 8000, 4500)
    window("W-204", l2, 11000, 4000, 4500, reference="")      # defect: empty Reference
    space("Meeting 2.01", l2, 250, 250, 3500, 11500, 7500)

    # ---- Roof --------------------------------------------------------------
    roof = ifcopenshell.api.root.create_entity(f, ifc_class="IfcRoof", name="RF-301", predefined_type="FLAT_ROOF")
    box(roof, 12400.0, 300.0, 8400.0, -200, -200, 7000)
    ifcopenshell.api.spatial.assign_container(f, relating_structure=roof_lvl, products=[roof])
    ifcopenshell.api.material.assign_material(f, products=[roof], material=concrete)
    add_pset(roof, "Pset_RoofCommon", {"IsExternal": True, "FireRating": "REI60", "Reference": "RF-301"})
    add_qto(roof, "Qto_RoofBaseQuantities", {"GrossArea": 104.16, "NetArea": 104.16, "ProjectedArea": 104.16})
    classify([roof], "Ss_30_40", "Roof covering systems")

    f.write(OUT)
    print(f"wrote {OUT} ({os.path.getsize(OUT)} bytes, {len(f.by_type('IfcProduct'))} products)")


if __name__ == "__main__":
    sys.exit(main())
