//! The script-visible state bag: ordered, serializable, typed.
//!
//! A script sees the world ONLY through (a) its function arguments and
//! (b) its [`ScriptVars`] bag. Keeping the bag a `BTreeMap` of a small
//! closed [`ScriptValue`] enum makes script state trivially
//! serializable (save system, P5-5) and byte-identical under replay —
//! Rhai's own `Dynamic` (which can hold closures, op-queues, ...) is
//! never persisted.

/// Kinds of values a script variable can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptValueKind {
    /// 64-bit float (the engine's scripting numeric type).
    Number,
    /// Boolean.
    Flag,
    /// Text.
    Text,
}

impl std::fmt::Display for ScriptValueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ScriptValueKind::Number => "number",
            ScriptValueKind::Flag => "flag",
            ScriptValueKind::Text => "text",
        })
    }
}

/// One script variable value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ScriptValue {
    /// 64-bit float.
    Number(f64),
    /// Boolean.
    Flag(bool),
    /// Text.
    Text(String),
}

impl ScriptValue {
    /// The value's kind.
    pub fn kind(&self) -> ScriptValueKind {
        match self {
            ScriptValue::Number(_) => ScriptValueKind::Number,
            ScriptValue::Flag(_) => ScriptValueKind::Flag,
            ScriptValue::Text(_) => ScriptValueKind::Text,
        }
    }

    /// The number, if this is one.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ScriptValue::Number(v) => Some(*v),
            _ => None,
        }
    }

    /// The flag, if this is one.
    pub fn as_flag(&self) -> Option<bool> {
        match self {
            ScriptValue::Flag(v) => Some(*v),
            _ => None,
        }
    }

    /// The text, if this is one.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ScriptValue::Text(v) => Some(v.as_str()),
            _ => None,
        }
    }
}

/// The error type for typed variable updates.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VarsError {
    /// The variable exists with a different kind.
    #[error("variable {0:?} holds a {1}; cannot store {2}")]
    TypeConflict(String, ScriptValueKind, ScriptValueKind),
}

/// The script-visible state bag.
///
/// Iteration is sorted by name (`BTreeMap`), so dumping the bag —
/// into a log, a save file, an evidence hash — is byte-identical for
/// identical content.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScriptVars {
    vars: std::collections::BTreeMap<String, ScriptValue>,
}

impl ScriptVars {
    /// New empty bag.
    pub fn new() -> ScriptVars {
        ScriptVars::default()
    }

    /// Reads a variable's value.
    pub fn get(&self, name: &str) -> Option<&ScriptValue> {
        self.vars.get(name)
    }

    /// Reads a variable as a number (kind-checked).
    pub fn get_number(&self, name: &str) -> Result<f64, VarsError> {
        match self.vars.get(name) {
            None => Ok(0.0),
            Some(ScriptValue::Number(v)) => Ok(*v),
            Some(other) => Err(VarsError::TypeConflict(
                name.to_string(),
                other.kind(),
                ScriptValueKind::Number,
            )),
        }
    }

    /// Reads a variable as a flag (kind-checked).
    pub fn get_flag(&self, name: &str) -> Result<bool, VarsError> {
        match self.vars.get(name) {
            None => Ok(false),
            Some(ScriptValue::Flag(v)) => Ok(*v),
            Some(other) => Err(VarsError::TypeConflict(
                name.to_string(),
                other.kind(),
                ScriptValueKind::Flag,
            )),
        }
    }

    /// Reads a variable as text (kind-checked; missing reads as "").
    pub fn get_text(&self, name: &str) -> Result<String, VarsError> {
        match self.vars.get(name) {
            None => Ok(String::new()),
            Some(ScriptValue::Text(v)) => Ok(v.clone()),
            Some(other) => Err(VarsError::TypeConflict(
                name.to_string(),
                other.kind(),
                ScriptValueKind::Text,
            )),
        }
    }

    /// Sets a number. An existing variable of another kind is a
    /// conflict (silent type-changing writes would make replay
    /// debugging miserable).
    pub fn set_number(&mut self, name: &str, value: f64) -> Result<(), VarsError> {
        self.checked_insert(name, ScriptValue::Number(value))
    }

    /// Sets a flag (kind-checked).
    pub fn set_flag(&mut self, name: &str, value: bool) -> Result<(), VarsError> {
        self.checked_insert(name, ScriptValue::Flag(value))
    }

    /// Sets text (kind-checked).
    pub fn set_text(&mut self, name: &str, value: impl Into<String>) -> Result<(), VarsError> {
        self.checked_insert(name, ScriptValue::Text(value.into()))
    }

    /// Inserts only when the kind agrees with any existing entry.
    fn checked_insert(&mut self, name: &str, value: ScriptValue) -> Result<(), VarsError> {
        match self.vars.get(name) {
            Some(existing) if existing.kind() != value.kind() => Err(VarsError::TypeConflict(
                name.to_string(),
                existing.kind(),
                value.kind(),
            )),
            _ => {
                self.vars.insert(name.to_string(), value);
                Ok(())
            }
        }
    }

    /// Removes a variable, returning it.
    pub fn remove(&mut self, name: &str) -> Option<ScriptValue> {
        self.vars.remove(name)
    }

    /// Number of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Iterates `(name, value)` sorted by name.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ScriptValue)> {
        self.vars.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_access_defaults() {
        let v = ScriptVars::new();
        assert!(v.is_empty());
        assert_eq!(v.get_number("missing").expect("default"), 0.0);
        assert!(!v.get_flag("missing").expect("default"));
        assert_eq!(v.get_text("missing").expect("default"), "");
    }

    #[test]
    fn set_get_roundtrip() {
        let mut v = ScriptVars::new();
        v.set_number("speed", 2.5).expect("set");
        v.set_flag("moving", true).expect("set");
        v.set_text("state", "walk").expect("set");
        assert_eq!(v.len(), 3);
        assert_eq!(v.get_number("speed").expect("get"), 2.5);
        assert!(v.get_flag("moving").expect("get"));
        assert_eq!(v.get_text("state").expect("get"), "walk");
        assert_eq!(
            v.get("speed").map(ScriptValue::kind),
            Some(ScriptValueKind::Number)
        );
    }

    #[test]
    fn kind_conflicts_rejected() {
        let mut v = ScriptVars::new();
        v.set_number("x", 1.0).expect("set");
        assert_eq!(
            v.set_flag("x", true),
            Err(VarsError::TypeConflict(
                "x".to_string(),
                ScriptValueKind::Number,
                ScriptValueKind::Flag
            ))
        );
        // The stored value is untouched.
        assert_eq!(v.get_number("x").expect("get"), 1.0);
    }

    #[test]
    fn iteration_is_name_sorted() {
        let mut v = ScriptVars::new();
        v.set_number("c", 3.0).expect("c");
        v.set_number("a", 1.0).expect("a");
        v.set_number("b", 2.0).expect("b");
        let names: Vec<&str> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn json_roundtrip_is_stable() {
        let mut v = ScriptVars::new();
        v.set_number("speed", 2.5).expect("set");
        v.set_flag("moving", true).expect("set");
        v.set_text("state", "walk").expect("set");
        let json = serde_json::to_string(&v).expect("serialize");
        let back: ScriptVars = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(v, back);
        // Same content -> same JSON bytes (deterministic manifest).
        let again = serde_json::to_string(&back).expect("serialize");
        assert_eq!(json, again);
    }
}
