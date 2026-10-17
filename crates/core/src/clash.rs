//! Clash detection — phase 2.
//!
//! The configuration (`clash.*`), the CLI command and the report section exist
//! so the feature can be added without changing any interface. Until the
//! geometry-level implementation lands, `run` returns an explicit
//! "not yet implemented" message: never a silent no-op.

use crate::config::Config;
use crate::i18n::Strings;
use crate::model::{ClashReport, Model};

/// Run clash detection between the configured sets.
pub fn run(model_a: &Model, model_b: Option<&Model>, config: &Config, strings: &Strings) -> ClashReport {
    let _ = (model_a, model_b);
    if !config.clash.enabled {
        return ClashReport { enabled: false, implemented: false, message: strings.get("clash.disabled"), clashes: vec![] };
    }
    ClashReport { enabled: true, implemented: false, message: strings.get("clash.not_implemented"), clashes: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_and_not_implemented_are_explicit() {
        let s = Strings::for_language(crate::config::Language::En);
        let m = Model::default();
        let r = run(&m, None, &Config::default(), &s);
        assert!(!r.enabled);
        assert!(!r.message.is_empty());
        let mut c = Config::default();
        c.clash.enabled = true;
        let r2 = run(&m, None, &c, &s);
        assert!(r2.enabled && !r2.implemented);
        assert!(r2.message.to_lowercase().contains("later version"));
    }
}
