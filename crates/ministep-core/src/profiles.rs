//! YAML synth-control profiles: pages of up to eight knobs, each sending one CC.
//!
//! Optional extras used by the GUI synth panel (ignored by the Python build):
//! a knob may carry `min`/`max`/`unit` for its display value and `default`
//! (0..127) for its initial position; a page may set `kind: envelope` to draw
//! its first four knobs as an ADSR shape.

use std::collections::BTreeMap;
use std::path::Path;

use serde_yaml::Value;

/// MiniLab's eight small encoders: CC -> knob number (Arturia/User and DAW programs).
pub const MINILAB3_KNOB_CONTROLS: [(u8, u8); 16] = [
    (74, 1), (71, 2), (76, 3), (77, 4), (93, 5), (18, 6), (19, 7), (16, 8),
    (86, 1), (87, 2), (89, 3), (90, 4), (110, 5), (111, 6), (116, 7), (117, 8),
];

pub fn minilab_knob(control: u8) -> Option<u8> {
    MINILAB3_KNOB_CONTROLS.iter().find(|(c, _)| *c == control).map(|&(_, k)| k)
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnobBinding {
    pub cc: u8,
    pub label: String,
    /// One-based.
    pub channel: u8,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub default: u8,
}

impl KnobBinding {
    /// Display value for a raw 0..127 position.
    pub fn display(&self, raw: u8) -> String {
        let v = self.min + (self.max - self.min) * f64::from(raw) / 127.0;
        let text = if (self.max - self.min).abs() >= 20.0 || v.fract().abs() < 1e-9 {
            format!("{}", v.round() as i64)
        } else {
            format!("{v:.2}")
        };
        if self.unit.is_empty() { text } else { format!("{text} {}", self.unit) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageKind {
    #[default]
    Knobs,
    Envelope,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlPage {
    pub name: String,
    pub kind: PageKind,
    pub knobs: BTreeMap<u8, KnobBinding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlProfile {
    pub name: String,
    pub output: Option<String>,
    pub pages: Vec<ControlPage>,
}

impl ControlProfile {
    pub fn page_names(&self) -> Vec<String> {
        self.pages.iter().map(|p| p.name.clone()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ProfileError(pub String);

fn err<T>(msg: impl Into<String>) -> Result<T, ProfileError> {
    Err(ProfileError(msg.into()))
}

fn string(v: Option<&Value>, field: &str) -> Result<String, ProfileError> {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => err(format!("{field} must be a non-empty string")),
    }
}

fn int_in(v: Option<&Value>, field: &str, lo: i64, hi: i64, what: &str) -> Result<i64, ProfileError> {
    match v.and_then(Value::as_i64) {
        Some(n) if (lo..=hi).contains(&n) => Ok(n),
        _ => err(format!("{field} must be an integer between {lo} and {hi}{what}")),
    }
}

fn key(map: &serde_yaml::Mapping, k: &str) -> Option<Value> {
    map.get(Value::String(k.into())).cloned()
}

pub fn parse_profile(text: &str) -> Result<ControlProfile, ProfileError> {
    let raw: Value = serde_yaml::from_str(text).map_err(|e| ProfileError(format!("invalid YAML: {e}")))?;
    let Value::Mapping(root) = raw else { return err("profile must be a YAML mapping") };
    let name = string(key(&root, "name").as_ref(), "name")?;
    let output = match key(&root, "output") {
        None | Some(Value::Null) => None,
        Some(v) => Some(string(Some(&v), "output")?),
    };
    let channel = match key(&root, "channel") {
        None => 1,
        v => int_in(v.as_ref(), "channel", 1, 16, "")?,
    };
    let Some(Value::Mapping(pages_raw)) = key(&root, "pages").filter(|p| p.as_mapping().is_some_and(|m| !m.is_empty())) else {
        return err("pages must be a non-empty mapping");
    };
    let mut pages = Vec::new();
    for (page_name, page_raw) in &pages_raw {
        let page_name = match page_name {
            Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
        };
        let Value::Mapping(page) = page_raw else { return err(format!("page {page_name:?} must be a mapping")) };
        let kind = match key(page, "kind").as_ref().and_then(Value::as_str) {
            Some("envelope") => PageKind::Envelope,
            _ => PageKind::Knobs,
        };
        let knobs_raw = match key(page, "knobs") {
            None | Some(Value::Null) => serde_yaml::Mapping::new(),
            Some(Value::Mapping(m)) => m,
            Some(_) => return err(format!("page {page_name:?}.knobs must be a mapping")),
        };
        let mut knobs = BTreeMap::new();
        for (k, b) in &knobs_raw {
            let knob = match k {
                Value::Number(n) => n.as_i64(),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            }
            .ok_or_else(|| ProfileError(format!("page {page_name:?} has a non-numeric knob key")))?;
            if !(1..=8).contains(&knob) {
                return err(format!("page {page_name:?} knob must be between 1 and 8"));
            }
            let Value::Mapping(b) = b else { return err(format!("page {page_name:?} knob {knob} must be a mapping")) };
            let field = format!("page {page_name:?} knob {knob}");
            let cc = int_in(key(b, "cc").as_ref(), &format!("{field}.cc"), 0, 127, "")?;
            let label = string(key(b, "label").as_ref(), &format!("{field}.label"))?;
            let ch = match key(b, "channel") {
                None => channel,
                v => int_in(v.as_ref(), "channel", 1, 16, "")?,
            };
            let num = |k: &str, d: f64| key(b, k).and_then(|v| v.as_f64()).unwrap_or(d);
            knobs.insert(
                knob as u8,
                KnobBinding {
                    cc: cc as u8,
                    label,
                    channel: ch as u8,
                    min: num("min", 0.0),
                    max: num("max", 127.0),
                    unit: key(b, "unit").as_ref().and_then(Value::as_str).unwrap_or_default().to_string(),
                    default: num("default", 0.0).clamp(0.0, 127.0) as u8,
                },
            );
        }
        pages.push(ControlPage { name: page_name, kind, knobs });
    }
    Ok(ControlProfile { name, output, pages })
}

pub fn load_profile(path: &Path) -> Result<ControlProfile, ProfileError> {
    let text = std::fs::read_to_string(path).map_err(|e| ProfileError(format!("cannot read {}: {e}", path.display())))?;
    parse_profile(&text).map_err(|e| ProfileError(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_miniverse_shape() {
        let p = parse_profile(
            "name: X\nchannel: 2\npages:\n  FILTER:\n    knobs:\n      1: { cc: 20, label: CUTOFF, max: 12000, unit: Hz }\n      2: { cc: 21, label: RES, channel: 3 }\n  EMPTY: {}\n",
        )
        .unwrap();
        assert_eq!(p.pages.len(), 2);
        let k = &p.pages[0].knobs[&1];
        assert_eq!((k.cc, k.channel), (20, 2));
        assert_eq!(p.pages[0].knobs[&2].channel, 3);
        assert_eq!(k.display(127), "12000 Hz");
        assert!(parse_profile("name: X\npages: {}").is_err());
        assert!(parse_profile("name: X\npages:\n  A:\n    knobs:\n      9: {cc: 1, label: x}").is_err());
    }
}
