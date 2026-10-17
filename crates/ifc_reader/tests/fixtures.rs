//! Integration tests against the public buildingSMART fixtures. They need a
//! Python with IfcOpenShell (see README); set VERIMODEL_SKIP_IFC_TESTS=1 to
//! skip explicitly (never silently).

use std::path::PathBuf;

use verimodel_ifc_reader::{inspect, read_model, ReadOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)
}

fn skip() -> bool {
    if std::env::var("VERIMODEL_SKIP_IFC_TESTS").is_ok() {
        eprintln!("VERIMODEL_SKIP_IFC_TESTS set: skipping IFC reader tests");
        return true;
    }
    false
}

#[test]
fn reads_ifc2x3_with_units_converted_to_metres() {
    if skip() {
        return;
    }
    let m = read_model(&fixture("bsi_ifc2x3_building_architecture.ifc"), &ReadOptions::default()).unwrap();
    assert_eq!(m.info.schema, "IFC2X3");
    assert_eq!(m.info.units.length_unit.as_deref(), Some("MILLIMETRE"));
    assert!((m.info.units.length_to_m - 0.001).abs() < 1e-12);
    assert_eq!(m.elements.len(), 19);
    assert_eq!(m.info.storeys, vec!["00 groundfloor"]);
    assert!(m.info.disciplines.contains(&"architecture".to_string()));
    let wall = m.elements.iter().find(|e| e.ifc_type == "IfcWall").unwrap();
    // 200 mm in the file -> 0.2 m
    assert!((wall.quantity("Width").unwrap() - 0.2).abs() < 1e-9, "{:?}", wall.quantities);
    assert_eq!(wall.storey.as_deref(), Some("00 groundfloor"));
    assert!(wall.geometry.is_some(), "geometry kernel should produce a shape for the wall");
    assert!(m.info.geometry_available);
    assert!(wall.is_a("IfcElement"));
}

#[test]
fn reads_ifc4_and_ifc4x3() {
    if skip() {
        return;
    }
    let s = read_model(&fixture("bsi_ifc4_building_structural.ifc"), &ReadOptions::default()).unwrap();
    assert_eq!(s.info.schema, "IFC4");
    assert_eq!(s.elements.len(), 21);
    assert!(s.info.disciplines.contains(&"structure".to_string()));
    let b = read_model(&fixture("bsi_ifc4x3_infra_bridge.ifc"), &ReadOptions::default()).unwrap();
    assert_eq!(b.info.schema, "IFC4X3");
    assert_eq!(b.elements.len(), 77);
    assert!(b.info.storeys.is_empty(), "bridge has no building storeys");
    assert!(b.info.disciplines.contains(&"civil".to_string()));
    assert!(b.info.spatial_tree.iter().flat_map(|r| r.walk()).any(|n| n.ifc_type == "IfcBridge"));
}

#[test]
fn inspect_is_fast_and_has_no_geometry() {
    if skip() {
        return;
    }
    let info = inspect(&fixture("bsi_ifc4_building_hvac.ifc")).unwrap();
    assert_eq!(info.schema, "IFC4");
    assert_eq!(info.element_count, 10);
    assert!(!info.geometry_available);
    assert!(info.disciplines.contains(&"mep".to_string()));
    assert_eq!(info.sha256.len(), 64);
}

#[test]
fn ids_results_are_returned_as_facts() {
    if skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ids = dir.path().join("t.ids");
    std::fs::write(&ids, r#"<?xml version="1.0" encoding="UTF-8"?>
<ids:ids xmlns:ids="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://standards.buildingsmart.org/IDS http://standards.buildingsmart.org/IDS/1.0/ids.xsd">
  <ids:info><ids:title>Reader test</ids:title></ids:info>
  <ids:specifications>
    <ids:specification name="Walls need FireRating" ifcVersion="IFC2X3 IFC4 IFC4X3_ADD2">
      <ids:applicability minOccurs="1" maxOccurs="unbounded"><ids:entity><ids:name><ids:simpleValue>IFCWALL</ids:simpleValue></ids:name></ids:entity></ids:applicability>
      <ids:requirements><ids:property dataType="IFCLABEL" cardinality="required"><ids:propertySet><ids:simpleValue>Pset_WallCommon</ids:simpleValue></ids:propertySet><ids:baseName><ids:simpleValue>FireRating</ids:simpleValue></ids:baseName></ids:property></ids:requirements>
    </ids:specification>
  </ids:specifications>
</ids:ids>"#).unwrap();
    let opts = ReadOptions { ids_path: Some(ids), geometry: false, ..Default::default() };
    let m = read_model(&fixture("bsi_ifc2x3_building_architecture.ifc"), &opts).unwrap();
    let r = m.ids_results.expect("ids results");
    assert_eq!(r.title.as_deref(), Some("Reader test"));
    assert_eq!(r.specifications.len(), 1);
    let s = &r.specifications[0];
    assert_eq!(s.applicable_count, 4);
    assert!(!s.passed);
    assert_eq!(s.requirements[0].failed_entities.len(), 4);
    assert!(s.requirements[0].description.contains("FireRating"));
}

#[test]
fn garbage_file_gives_plain_language_error() {
    if skip() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bad.ifc");
    std::fs::write(&p, "this is not an ifc file").unwrap();
    let err = read_model(&p, &ReadOptions::default()).unwrap_err();
    assert_eq!(err.code, "parse_failed", "{err:?}");
    assert!(!err.hint.is_empty());
}
