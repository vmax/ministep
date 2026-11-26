//! Named pattern persistence: one JSON file per pattern, atomic writes.
//!
//! Melodic and drum patterns live in `melodic/` and `drums/` sub-directories.
//! Older combined files in the root directory are listed for both kinds and
//! loading one takes only the matching section.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::state::{AppState, FORMAT_VERSION, TRACK_FORMAT_VERSION};

pub const PATTERN_SUFFIX: &str = ".json";
pub const MAX_NAME_LENGTH: usize = 64;

/// A user-facing persistence failure, ready for the status line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PatternError(pub String);

impl From<String> for PatternError {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternKind {
    Melodic,
    Drums,
}

impl PatternKind {
    pub const ALL: [PatternKind; 2] = [PatternKind::Melodic, PatternKind::Drums];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Melodic => "melodic",
            Self::Drums => "drums",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Melodic => "melodic pattern",
            Self::Drums => "drum pattern",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

pub fn default_config_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".config").join("ministep")
}

pub fn default_patterns_dir() -> PathBuf {
    default_config_dir().join("patterns")
}

/// Reduce a display name to a filename stem that cannot leave the directory.
pub fn sanitize_pattern_name(name: &str) -> Result<String, PatternError> {
    let replaced = name.trim().replace(['/', '\\'], "_");
    let mut stem = String::with_capacity(replaced.len());
    let mut in_unsafe = false;
    for c in replaced.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            stem.push(c);
            in_unsafe = false;
        } else if !in_unsafe {
            stem.push('_');
            in_unsafe = true;
        }
    }
    let mut stem = stem.trim_matches(|c| c == '.' || c == '_').to_string();
    while stem.contains("__") {
        stem = stem.replace("__", "_");
    }
    if let Some(s) = stem.strip_suffix(PATTERN_SUFFIX) {
        stem = s.trim_end_matches(['.', '_']).to_string();
    }
    if stem.is_empty() {
        return Err(PatternError("Pattern name must contain a letter or digit".into()));
    }
    Ok(stem.chars().take(MAX_NAME_LENGTH).collect())
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn stem(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Write pretty JSON (2-space indent, trailing newline) via temp file + fsync + rename.
pub fn write_json_atomic(path: &Path, data: &Value) -> Result<(), PatternError> {
    let fail = |e: std::io::Error| PatternError(format!("Cannot write {}: {e}", file_name(path)));
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(fail)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{}.", file_name(path)))
        .tempfile_in(parent)
        .map_err(fail)?;
    let mut text = serde_json::to_string_pretty(data).map_err(|e| PatternError(e.to_string()))?;
    text.push('\n');
    tmp.write_all(text.as_bytes()).map_err(fail)?;
    tmp.as_file().sync_all().map_err(fail)?;
    tmp.persist(path).map_err(|e| fail(e.error))?;
    Ok(())
}

pub fn read_json(path: &Path) -> Result<Value, PatternError> {
    let text = fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => PatternError(format!("Pattern not found: {}", stem(path))),
        _ => PatternError(format!("Cannot read {}: {e}", file_name(path))),
    })?;
    serde_json::from_str(&text).map_err(|e| PatternError(format!("{} is not valid JSON: {e}", file_name(path))))
}

/// Return the `kind` section of any pattern file MiniStep has ever written.
pub fn extract_section(kind: PatternKind, data: &Value, source: &str) -> Result<Value, PatternError> {
    let obj = data.as_object().ok_or_else(|| PatternError(format!("{source} must contain a JSON object")))?;
    let version = obj.get("version").map_or(Some(1), Value::as_i64);
    if let Some(file_type) = obj.get("type").filter(|t| !t.is_null()) {
        let parsed = file_type.as_str().and_then(PatternKind::parse);
        let Some(file_kind) = parsed else {
            return Err(PatternError(format!("{source}: unknown pattern type {file_type}")));
        };
        if file_kind != kind {
            return Err(PatternError(format!("{source} is a {}, not a {}", file_kind.label(), kind.label())));
        }
        if version != Some(TRACK_FORMAT_VERSION) {
            return Err(PatternError(format!(
                "{source}: unsupported {} version {} (this MiniStep reads version {TRACK_FORMAT_VERSION})",
                kind.label(),
                obj.get("version").map_or("None".into(), Value::to_string)
            )));
        }
        return Ok(data.clone());
    }
    match version {
        Some(1) if kind == PatternKind::Drums => {
            Err(PatternError(format!("{source} has no drum section (old melodic-only file)")))
        }
        Some(1) => Ok(data.clone()),
        Some(FORMAT_VERSION) => obj
            .get(kind.as_str())
            .filter(|s| !s.is_null())
            .cloned()
            .ok_or_else(|| PatternError(format!("{source} has no {} section", kind.as_str()))),
        _ => Err(PatternError(format!(
            "{source}: unsupported pattern version {}",
            obj.get("version").map_or("None".into(), Value::to_string)
        ))),
    }
}

/// Save and load named melodic and drum patterns under one directory.
#[derive(Debug, Clone)]
pub struct PatternStore {
    pub directory: PathBuf,
}

impl Default for PatternStore {
    fn default() -> Self {
        Self::new(default_patterns_dir())
    }
}

fn normalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl PatternStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    pub fn kind_directory(&self, kind: PatternKind) -> PathBuf {
        self.directory.join(kind.as_str())
    }

    pub fn path_for(&self, kind: PatternKind, name: &str) -> Result<PathBuf, PatternError> {
        // Sanitised stems never contain separators, so the path cannot escape.
        Ok(self.kind_directory(kind).join(format!("{}{PATTERN_SUFFIX}", sanitize_pattern_name(name)?)))
    }

    fn legacy_path(&self, name: &str) -> Option<PathBuf> {
        let path = self.directory.join(format!("{}{PATTERN_SUFFIX}", sanitize_pattern_name(name).ok()?));
        path.is_file().then_some(path)
    }

    fn stems(directory: &Path) -> BTreeSet<String> {
        let Ok(entries) = fs::read_dir(directory) else { return BTreeSet::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
            .map(|p| stem(&p))
            .filter(|s| !s.starts_with('.'))
            .collect()
    }

    /// Names saved as `kind` plus legacy combined files.
    pub fn list_names(&self, kind: PatternKind) -> Vec<String> {
        let mut names = Self::stems(&self.kind_directory(kind));
        names.extend(Self::stems(&self.directory));
        names.into_iter().collect()
    }

    pub fn exists(&self, kind: PatternKind, name: &str) -> bool {
        self.path_for(kind, name).is_ok_and(|p| p.is_file()) || self.legacy_path(name).is_some()
    }

    /// Write `state`'s `kind` track as `name`; return the sanitised stored name.
    pub fn save(&self, kind: PatternKind, state: &AppState, name: &str) -> Result<String, PatternError> {
        let path = self.path_for(kind, name)?;
        let mut data = Map::new();
        data.insert("version".into(), json!(TRACK_FORMAT_VERSION));
        data.insert("type".into(), json!(kind.as_str()));
        let display = if name.trim().is_empty() { stem(&path) } else { name.trim().to_string() };
        data.insert("name".into(), json!(display));
        let section = match kind {
            PatternKind::Melodic => state.melodic_to_json(),
            PatternKind::Drums => state.drums.to_json(),
        };
        data.extend(section);
        write_json_atomic(&path, &Value::Object(data))?;
        Ok(stem(&path))
    }

    /// Locate `name` for `kind` (own directory first, then legacy) and extract it.
    pub fn read_section(&self, kind: PatternKind, name: &str) -> Result<Value, PatternError> {
        let mut path = self.path_for(kind, name)?;
        if !path.is_file() {
            path = self.legacy_path(name).ok_or_else(|| PatternError(format!("Pattern not found: {}", stem(&path))))?;
        }
        extract_section(kind, &read_json(&path)?, &file_name(&path))
    }

    /// Apply the named pattern to that one track of `state`; untouched on failure.
    pub fn load(&self, kind: PatternKind, state: &mut AppState, name: &str) -> Result<String, PatternError> {
        let section = self.read_section(kind, name)?;
        let result = match kind {
            PatternKind::Melodic => state.apply_melodic(&section),
            PatternKind::Drums => state.apply_drums(&section),
        };
        result.map_err(|e| PatternError(format!("{name}: {e}")))?;
        Ok(stem(&self.path_for(kind, name)?))
    }

    pub fn delete(&self, kind: PatternKind, name: &str) -> Result<(), PatternError> {
        let path = self.path_for(kind, name)?;
        fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => PatternError(format!("Pattern not found: {}", stem(&path))),
            _ => PatternError(format!("Cannot delete {}: {e}", file_name(&path))),
        })
    }

    pub fn rename(&self, kind: PatternKind, old: &str, new: &str) -> Result<String, PatternError> {
        let (source, target) = (self.path_for(kind, old)?, self.path_for(kind, new)?);
        if !source.is_file() {
            return Err(PatternError(format!("Pattern not found: {}", stem(&source))));
        }
        if target.exists() && normalize(&target) != normalize(&source) {
            return Err(PatternError(format!("Pattern already exists: {}", stem(&target))));
        }
        let mut data = read_json(&source)?;
        if let Some(obj) = data.as_object_mut() {
            let display = if new.trim().is_empty() { stem(&target) } else { new.trim().to_string() };
            obj.insert("name".into(), json!(display));
        }
        write_json_atomic(&target, &data)?;
        if normalize(&target) != normalize(&source) {
            fs::remove_file(&source).map_err(|e| PatternError(format!("Cannot rename {}: {e}", file_name(&source))))?;
        }
        Ok(stem(&target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_matches_python() {
        assert_eq!(sanitize_pattern_name("Egypt 01").unwrap(), "Egypt_01");
        assert_eq!(sanitize_pattern_name("../x").unwrap(), "x");
        assert_eq!(sanitize_pattern_name(".hidden").unwrap(), "hidden");
        assert_eq!(sanitize_pattern_name("a  //  b").unwrap(), "a_b");
        assert_eq!(sanitize_pattern_name("song.json").unwrap(), "song");
        assert!(sanitize_pattern_name(" ../ ").is_err());
        assert_eq!(sanitize_pattern_name(&"x".repeat(100)).unwrap().len(), 64);
    }
}
