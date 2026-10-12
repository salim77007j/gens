//! Data-driven content documents (Phase 6).
//!
//! A [`DataDocument`] is the engine's **scene / prefab** format: a
//! versioned, validated JSON document that describes entities with
//! engine-standard transforms and optional script bindings. It rides
//! the same pipeline as every other asset — registry identity is
//! FNV(canonical key), content is fingerprinted with FNV-1a 64, and a
//! changed document bumps the entry version — so manifests, saves and
//! (Phase 6) the network all agree on *which* document a world was
//! built from.
//!
//! Determinism contract: parsing the same bytes always produces the
//! same document (`PartialEq` is derived and NaN can never pass
//! validation), and [`canonical_json`] of equal documents is
//! byte-identical (struct field order, `BTreeMap` for variables —
//! never `HashMap`). File I/O happens only in the `*_file` helpers;
//! everything below them is pure.
//!
//! Scope, stated honestly (R-023): a data document covers the
//! engine-standard vocabulary (name, TRS, script + variables). It is
//! **not** a reflection layer for arbitrary game components — those
//! still travel through script variables. The format is versioned so
//! a future reflection layer can extend it without breaking readers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{AssetError, DataError};
use crate::registry::{AssetId, AssetKind, AssetRegistry};

/// The only data-document format version this engine reads.
///
/// Readers reject anything else loudly (a future version must not be
/// silently misread with fields missing).
pub const DATA_FORMAT_VERSION: u32 = 1;

/// Rotation quaternions must satisfy `|norm - 1| <= ROTATION_NORM_TOL`.
///
/// Loose enough for 6-decimal-digit authored JSON; tight enough that
/// downstream normalization never surprises anyone.
pub const ROTATION_NORM_TOL: f32 = 1e-4;

/// A versioned scene / prefab document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataDocument {
    /// Must equal [`DATA_FORMAT_VERSION`].
    pub format_version: u32,
    /// Document name (non-empty; appears in manifests and evidence).
    pub name: String,
    /// Entities in authored order. Order is preserved (it is the spawn
    /// order at the runtime bridge) and is part of document identity.
    #[serde(default)]
    pub entities: Vec<DataEntity>,
}

/// One entity described by a data document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataEntity {
    /// Entity name, unique within the document (the bridge looks
    /// entities up by name when scripts cross-reference).
    pub name: String,
    /// Whether the entity spawns with the `Active` marker (default true).
    #[serde(default = "default_true")]
    pub active: bool,
    /// Optional free-form tag, same vocabulary as the save format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Local translation, `[x, y, z]`, all finite.
    #[serde(default = "default_translation")]
    pub translation: [f32; 3],
    /// Local rotation as a unit quaternion `[x, y, z, w]`.
    #[serde(default = "default_rotation")]
    pub rotation: [f32; 4],
    /// Local scale, all components finite and non-zero.
    #[serde(default = "default_scale")]
    pub scale: [f32; 3],
    /// Optional script binding (registry key + entry fn + initial vars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<DataScript>,
}

/// Initial script state attached to a [`DataEntity`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataScript {
    /// Registry key of the script asset (e.g. `"scripts/patrol.rhai"`).
    pub script: String,
    /// Entry function name called per tick by the scripting host.
    pub entry: String,
    /// Initial variable values, ordered (`BTreeMap`) so the canonical
    /// JSON — and therefore the content hash — is byte-stable.
    #[serde(default)]
    pub vars: BTreeMap<String, DataVar>,
}

/// A script variable as seen by the data format.
///
/// Deliberately a *mirror* vocabulary (`f64`/`bool`/`String`), not a
/// live scripting type: gens-assets stays free of the scripting
/// engine, and the runtime bridge (gens-runtime) converts into
/// `ScriptValue` with kind checks — the same decoupling the save
/// format uses (ADR-0021).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DataVar {
    /// Floating-point number (f64 on the wire, like script numbers).
    Number(f64),
    /// Boolean flag.
    Flag(bool),
    /// Text value.
    Text(String),
}

impl DataVar {
    /// Human-readable kind name (used in bridge diagnostics).
    pub fn kind_name(&self) -> &'static str {
        match self {
            DataVar::Number(_) => "number",
            DataVar::Flag(_) => "flag",
            DataVar::Text(_) => "text",
        }
    }
}

fn default_translation() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn default_true() -> bool {
    true
}

fn default_rotation() -> [f32; 4] {
    // Identity quaternion (x, y, z, w).
    [0.0, 0.0, 0.0, 1.0]
}

fn default_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl DataEntity {
    /// A helper for tests and programmatic construction: a fully
    /// defaulted entity with `name` and nothing else.
    pub fn named(name: impl Into<String>) -> DataEntity {
        DataEntity {
            name: name.into(),
            active: true,
            tag: None,
            translation: default_translation(),
            rotation: default_rotation(),
            scale: default_scale(),
            script: None,
        }
    }
}

impl DataDocument {
    /// Validates the document (called by every importer).
    pub fn validate(&self) -> Result<(), DataError> {
        if self.format_version != DATA_FORMAT_VERSION {
            return Err(DataError::UnsupportedVersion {
                found: self.format_version,
            });
        }
        if self.name.is_empty() {
            return Err(DataError::EmptyDocumentName);
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, entity) in self.entities.iter().enumerate() {
            if entity.name.is_empty() {
                return Err(DataError::EmptyEntityName { index });
            }
            if !seen.insert(entity.name.as_str()) {
                return Err(DataError::DuplicateEntityName {
                    name: entity.name.clone(),
                });
            }
            if !entity.translation.iter().all(|v| v.is_finite()) {
                return Err(DataError::NonFiniteTranslation {
                    name: entity.name.clone(),
                });
            }
            if !entity.rotation.iter().all(|v| v.is_finite()) {
                return Err(DataError::NonFiniteRotation {
                    name: entity.name.clone(),
                });
            }
            let (x, y, z, w) = (
                entity.rotation[0] as f64,
                entity.rotation[1] as f64,
                entity.rotation[2] as f64,
                entity.rotation[3] as f64,
            );
            let norm = (x * x + y * y + z * z + w * w).sqrt() as f32;
            if (norm - 1.0).abs() > ROTATION_NORM_TOL {
                return Err(DataError::NonUnitRotation {
                    name: entity.name.clone(),
                    norm,
                    tol: ROTATION_NORM_TOL,
                });
            }
            if !entity.scale.iter().all(|v| v.is_finite()) {
                return Err(DataError::NonFiniteScale {
                    name: entity.name.clone(),
                });
            }
            if entity.scale.contains(&0.0) {
                return Err(DataError::ZeroScale {
                    name: entity.name.clone(),
                });
            }
            if let Some(script) = &entity.script {
                if script.script.is_empty() {
                    return Err(DataError::EmptyScriptKey {
                        name: entity.name.clone(),
                    });
                }
                if script.entry.is_empty() {
                    return Err(DataError::EmptyScriptEntry {
                        name: entity.name.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Canonical JSON: field order is the struct order and variable
    /// maps are `BTreeMap`s, so equal documents serialize to
    /// byte-identical strings (asserted by tests).
    pub fn canonical_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

/// Parses and validates a data document from bytes.
///
/// The `key` only appears in error diagnostics; identity still comes
/// from the registry.
pub fn parse_data_document(key: &str, bytes: &[u8]) -> Result<DataDocument, AssetError> {
    let doc: DataDocument = serde_json::from_slice(bytes).map_err(|source| AssetError::Data {
        key: key.to_string(),
        source: DataError::Json { source },
    })?;
    doc.validate().map_err(|source| AssetError::Data {
        key: key.to_string(),
        source,
    })?;
    Ok(doc)
}

/// Parses + validates + registers a data document in one step.
///
/// Validation runs BEFORE registration, so invalid content never
/// enters the registry (a registered-but-invalid document would poison
/// every manifest built from it). Re-import bumps the version only
/// when the bytes changed — the registry's own rule.
pub fn import_data_document(
    registry: &mut AssetRegistry,
    key: &str,
    bytes: &[u8],
) -> Result<(AssetId, DataDocument), AssetError> {
    let doc = parse_data_document(key, bytes)?;
    let (id, _entry) = registry.import(key, AssetKind::Data, bytes)?;
    Ok((id, doc))
}

/// [`import_data_document`] from a file path.
///
/// The key is the path as given, canonicalized to forward slashes —
/// the same rule [`AssetRegistry::import_file`] applies.
pub fn import_data_document_file(
    registry: &mut AssetRegistry,
    path: impl AsRef<std::path::Path>,
) -> Result<(AssetId, DataDocument), AssetError> {
    let path = path.as_ref();
    let key = path.to_string_lossy().replace('\\', "/");
    let bytes = std::fs::read(path).map_err(|source| AssetError::Io {
        key: key.clone(),
        source,
    })?;
    import_data_document(registry, &key, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "format_version": 1,
        "name": "patrol-scene",
        "entities": [
            {
                "name": "agent",
                "translation": [1.0, 0.5, -2.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "scale": [1.0, 1.0, 1.0],
                "script": {
                    "script": "scripts/patrol.rhai",
                    "entry": "tick",
                    "vars": {
                        "speed": {"kind": "number", "value": 2.0},
                        "phase": {"kind": "flag", "value": true},
                        "label": {"kind": "text", "value": "a"}
                    }
                }
            },
            {
                "name": "floor",
                "translation": [0.0, -1.0, 0.0]
            }
        ]
    }"#;

    fn parse(s: &str) -> DataDocument {
        parse_data_document("test/scene.json", s.as_bytes()).expect("valid document should parse")
    }

    #[test]
    fn parses_valid_document_with_defaults() {
        let doc = parse(VALID);
        assert_eq!(doc.format_version, DATA_FORMAT_VERSION);
        assert_eq!(doc.name, "patrol-scene");
        assert_eq!(doc.entities.len(), 2);
        let agent = &doc.entities[0];
        assert_eq!(agent.name, "agent");
        assert_eq!(agent.translation, [1.0, 0.5, -2.0]);
        assert_eq!(agent.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(agent.scale, [1.0, 1.0, 1.0]);
        let script = agent.script.as_ref().expect("agent has a script");
        assert_eq!(script.script, "scripts/patrol.rhai");
        assert_eq!(script.entry, "tick");
        assert_eq!(script.vars.len(), 3);
        // floor: defaults applied for the omitted fields
        let floor = &doc.entities[1];
        assert_eq!(floor.translation, [0.0, -1.0, 0.0]);
        assert_eq!(floor.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(floor.scale, [1.0, 1.0, 1.0]);
        assert!(floor.active, "active defaults to true");
        assert!(floor.tag.is_none());
        assert!(floor.script.is_none());
    }

    #[test]
    fn canonical_json_is_deterministic_and_round_trips() {
        let a = parse(VALID);
        // Same content, different key order + whitespace.
        let shuffled = r#"{"entities":[{"name":"agent","scale":[1.0,1.0,1.0],"rotation":[0.0,0.0,0.0,1.0],"translation":[1.0,0.5,-2.0],"script":{"vars":{"speed":{"value":2.0,"kind":"number"},"phase":{"kind":"flag","value":true},"label":{"kind":"text","value":"a"}},"entry":"tick","script":"scripts/patrol.rhai"}},{"name":"floor","translation":[0.0,-1.0,0.0]}],"name":"patrol-scene","format_version":1}"#;
        let b = parse(shuffled);
        assert_eq!(a, b, "equal documents regardless of JSON key order");
        let ca = a.canonical_json().expect("canonical serialize");
        let cb = b.canonical_json().expect("canonical serialize");
        assert_eq!(ca, cb, "canonical form is byte-identical");
        // Vars serialize in BTreeMap order: label, phase, speed.
        let label_pos = ca.find("\"label\"").expect("label present");
        let phase_pos = ca.find("\"phase\"").expect("phase present");
        let speed_pos = ca.find("\"speed\"").expect("speed present");
        assert!(
            label_pos < phase_pos && phase_pos < speed_pos,
            "sorted vars"
        );
        let reparsed: DataDocument = serde_json::from_str(&ca).expect("canonical form re-parses");
        assert_eq!(reparsed, a);
    }

    #[test]
    fn rejects_unsupported_version() {
        let doc = DataDocument {
            format_version: 2,
            name: "future".into(),
            entities: vec![],
        };
        assert!(matches!(
            doc.validate(),
            Err(DataError::UnsupportedVersion { found: 2 })
        ));
    }

    #[test]
    fn rejects_empty_document_name() {
        let doc = DataDocument {
            format_version: 1,
            name: String::new(),
            entities: vec![],
        };
        assert!(matches!(doc.validate(), Err(DataError::EmptyDocumentName)));
    }

    #[test]
    fn rejects_empty_entity_name_with_index() {
        let doc = DataDocument {
            format_version: 1,
            name: "s".into(),
            entities: vec![DataEntity::named("ok"), DataEntity::named("")],
        };
        assert!(matches!(
            doc.validate(),
            Err(DataError::EmptyEntityName { index: 1 })
        ));
    }

    #[test]
    fn rejects_duplicate_entity_names() {
        let doc = DataDocument {
            format_version: 1,
            name: "s".into(),
            entities: vec![DataEntity::named("twice"), DataEntity::named("twice")],
        };
        assert!(matches!(
            doc.validate(),
            Err(DataError::DuplicateEntityName { name }) if name == "twice"
        ));
    }

    #[test]
    fn tag_and_active_round_trip() {
        let json = r#"{"format_version":1,"name":"s","entities":[{"name":"a","active":false,"tag":"obstacle"}]}"#;
        let doc = parse(json);
        let e = &doc.entities[0];
        assert!(!e.active);
        assert_eq!(e.tag.as_deref(), Some("obstacle"));
        // Canonical form keeps the fields stable for re-parse.
        let ca = doc.canonical_json().unwrap();
        let reparsed: DataDocument = serde_json::from_str(&ca).unwrap();
        assert_eq!(reparsed, doc);
    }

    #[test]
    fn rejects_non_unit_rotation() {
        let mut doc = parse(VALID);
        doc.entities[0].rotation = [0.5, 0.0, 0.0, 0.0]; // norm 0.5
        assert!(matches!(
            doc.validate(),
            Err(DataError::NonUnitRotation { norm, .. }) if (norm - 0.5).abs() < 1e-6
        ));
    }

    #[test]
    fn rejects_zero_scale() {
        let mut doc = parse(VALID);
        doc.entities[0].scale = [1.0, 0.0, 1.0];
        assert!(matches!(
            doc.validate(),
            Err(DataError::ZeroScale { name }) if name == "agent"
        ));
    }

    #[test]
    fn validate_rejects_non_finite_directly() {
        // JSON itself cannot carry NaN/inf (serde_json rejects them at
        // parse), so this path is defense in depth for documents built
        // programmatically or parsed by other dialects.
        let mut doc = parse(VALID);
        doc.entities[0].translation = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            doc.validate(),
            Err(DataError::NonFiniteTranslation { name }) if name == "agent"
        ));
        let mut doc = parse(VALID);
        doc.entities[0].rotation = [f32::INFINITY, 0.0, 0.0, 1.0];
        assert!(matches!(
            doc.validate(),
            Err(DataError::NonFiniteRotation { name }) if name == "agent"
        ));
        let mut doc = parse(VALID);
        doc.entities[0].scale = [1.0, 1.0, f32::NEG_INFINITY];
        assert!(matches!(
            doc.validate(),
            Err(DataError::NonFiniteScale { name }) if name == "agent"
        ));
    }

    #[test]
    fn json_out_of_range_number_fails_at_parse_with_clear_error() {
        // 1e400 overflows f64: surfaced as AssetError::Data{Json}, not a panic.
        let err = parse_data_document(
            "test/overflow.json",
            br#"{"format_version":1,"name":"s","entities":[{"name":"e","translation":[1e400,0.0,0.0]}]}"#,
        )
        .expect_err("overflow must fail");
        assert!(err.to_string().contains("data import failed"), "{err}");
    }

    #[test]
    fn rejects_empty_script_key_and_entry() {
        let mut doc = parse(VALID);
        doc.entities[0].script.as_mut().unwrap().script = String::new();
        assert!(matches!(
            doc.validate(),
            Err(DataError::EmptyScriptKey { name }) if name == "agent"
        ));
        let mut doc = parse(VALID);
        doc.entities[0].script.as_mut().unwrap().entry = String::new();
        assert!(matches!(
            doc.validate(),
            Err(DataError::EmptyScriptEntry { name }) if name == "agent"
        ));
    }

    #[test]
    fn registry_integration_identity_version_and_no_registration_on_invalid() {
        let mut registry = AssetRegistry::new();
        let (id, doc) = import_data_document(&mut registry, "scenes/patrol.json", VALID.as_bytes())
            .expect("valid import");
        assert_eq!(id, AssetId::from_key("scenes/patrol.json"));
        let entry = registry.entry(id).expect("registered");
        assert_eq!(entry.kind, AssetKind::Data);
        assert_eq!(entry.version, 1);
        assert_eq!(entry.size, VALID.len());
        assert_eq!(entry.content_hash, crate::fnv1a64(VALID.as_bytes()));

        // Unchanged re-import keeps version 1.
        let (id2, _) = import_data_document(&mut registry, "scenes/patrol.json", VALID.as_bytes())
            .expect("re-import");
        assert_eq!(id2, id);
        assert_eq!(registry.entry(id).unwrap().version, 1);

        // Changed bytes bump to version 2.
        let mut changed = doc.clone();
        changed.entities[0].translation = [3.0, 0.0, 0.0];
        let bytes2 = changed.canonical_json().unwrap().into_bytes();
        let _ = import_data_document(&mut registry, "scenes/patrol.json", &bytes2)
            .expect("changed import");
        assert_eq!(registry.entry(id).unwrap().version, 2);

        // Invalid content is never registered.
        let bad =
            r#"{"format_version":1,"name":"bad","entities":[{"name":"a","scale":[0.0,1.0,1.0]}]}"#;
        assert!(import_data_document(&mut registry, "scenes/bad.json", bad.as_bytes()).is_err());
        assert!(registry.entry_by_key("scenes/bad.json").is_none());
    }

    #[test]
    fn kind_change_for_existing_key_is_rejected() {
        let mut registry = AssetRegistry::new();
        registry
            .import("dual/purpose", AssetKind::Script, b"fn tick() {}")
            .expect("script import");
        let err = import_data_document(&mut registry, "dual/purpose", VALID.as_bytes())
            .expect_err("kind change must fail");
        assert!(err.to_string().contains("imported as script"), "{err}");
    }

    #[test]
    fn file_import_round_trip() {
        let mut registry = AssetRegistry::new();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gens_data_test_{}_{}.json",
            crate::fnv1a64(b"file_import_round_trip"),
            std::process::id()
        ));
        std::fs::write(&path, VALID.as_bytes()).expect("write fixture");
        let result = import_data_document_file(&mut registry, &path);
        let _ = std::fs::remove_file(&path);
        let (id, doc) = result.expect("file import");
        assert_eq!(doc.name, "patrol-scene");
        assert!(registry.entry(id).is_some());
    }
}
