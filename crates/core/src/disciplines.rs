//! Discipline inference from entity types, driven by `data/disciplines.yaml`.

use std::collections::BTreeMap;

use crate::model::Element;

const DISCIPLINES_YAML: &str = include_str!("../data/disciplines.yaml");

fn table() -> BTreeMap<String, Vec<String>> {
    serde_yaml::from_str(DISCIPLINES_YAML).expect("built-in disciplines.yaml must be valid")
}

/// Disciplines present in the element list, in alphabetical order.
pub fn detect(elements: &[Element]) -> Vec<String> {
    let t = table();
    let mut out = Vec::new();
    for (discipline, types) in t {
        if elements.iter().any(|e| types.iter().any(|t| e.is_a(t))) {
            out.push(discipline);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_by_ancestor() {
        let e = Element {
            ifc_type: "IfcDuctSegment".into(),
            ancestors: vec![
                "IfcFlowSegment".into(),
                "IfcDistributionFlowElement".into(),
                "IfcDistributionElement".into(),
            ],
            ..Default::default()
        };
        assert_eq!(detect(&[e]), vec!["mep".to_string()]);
    }
}
