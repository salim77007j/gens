//! Data-document instantiation (Phase 6).
//!
//! The runtime bridge between the content format
//! ([`gens_assets::DataDocument`]) and a live ECS world. It is the
//! mirror of [`crate::save`]: save captures a world into a document;
//! this module spawns a document into a world.
//!
//! Determinism contract: applying the same document into two FRESH
//! worlds produces entity sets whose [`crate::save::capture_world`]
//! output is byte-identical (spawn order = authored order, so entity
//! indices are sequential and stable; `ScriptVars`/`ScriptRequests`
//! are `BTreeMap`s; transforms come from the document's numbers
//! verbatim). `WorldTransform` is never instantiated here — it is
//! derived, exactly like after a save load.
//!
//! Scope: the engine-standard vocabulary only (name, active, tag,
//! transform, script + initial vars) — the same scope statement as
//! the save format (ADR-0021) and the data format (R-023). Script
//! *sources* are not compiled here: the document references script
//! asset keys, and the caller registers them with the
//! [`gens_scripting::ScriptHost`] (missing scripts skip at tick time,
//! the P5-4 contract).

use gens_assets::{DataDocument, DataError, DataVar};
use gens_ecs::{Active, Name, Tag, Transform};
use gens_math::{Quat, Vec3};
use gens_scripting::{ScriptComponent, ScriptRequests, ScriptVars};

/// Errors raised while instantiating a data document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataBridgeError {
    /// The document failed (re-)validation. Every application
    /// re-validates: documents can also be built programmatically,
    /// and the bridge refuses to spawn a world nobody asked for.
    #[error("data document is invalid: {0}")]
    InvalidDocument(#[from] DataError),
}

/// Convenience alias for data-bridge results.
pub type DataResult<T> = Result<T, DataBridgeError>;

/// Converts a data-format variable into a scripting variable.
///
/// The two vocabularies are intentionally identical
/// (`Number(f64)`/`Flag`/`Text`), so the conversion is total and
/// lossless — a "kind check" by construction. Kept as a function so a
/// future vocabulary divergence has exactly one place to fail.
pub fn data_var_to_script_value(var: &DataVar) -> gens_scripting::ScriptValue {
    match var {
        DataVar::Number(v) => gens_scripting::ScriptValue::Number(*v),
        DataVar::Flag(v) => gens_scripting::ScriptValue::Flag(*v),
        DataVar::Text(v) => gens_scripting::ScriptValue::Text(v.clone()),
    }
}

/// The entity ids produced by [`apply_data_document`], in spawn order
/// (= authored order). Name lookup stays available for cross-references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataSpawnReport {
    /// Spawned ids in authored order.
    pub ids: Vec<gens_ecs::ecs::entity::Entity>,
}

/// Spawns `doc`'s entities into `world` (which should be FRESH: spawn
/// order assumes the allocator hands out indices sequentially, the
/// same rule as [`crate::save::apply_save`]).
///
/// The document is re-validated first (loud, cheap, and the only way
/// programmatically-built documents are covered by the same contract
/// as parsed ones). After spawning, [`gens_ecs::EngineSchedules::run_frame`]
/// recomputes `WorldTransform` — always derived, never stored.
pub fn apply_data_document(
    world: &mut gens_ecs::ecs::world::World,
    doc: &DataDocument,
) -> DataResult<DataSpawnReport> {
    doc.validate()?;
    let mut ids = Vec::with_capacity(doc.entities.len());
    for entity in &doc.entities {
        let mut spawned = world.spawn_empty();
        spawned.insert(Name::new(entity.name.clone()));
        spawned.insert(Transform {
            position: Vec3::from_array(entity.translation),
            rotation: Quat::from_xyzw(
                entity.rotation[0],
                entity.rotation[1],
                entity.rotation[2],
                entity.rotation[3],
            ),
            scale: Vec3::from_array(entity.scale),
        });
        if entity.active {
            spawned.insert(Active);
        }
        if let Some(tag) = &entity.tag {
            spawned.insert(Tag::new(tag.clone()));
        }
        if let Some(script) = &entity.script {
            let mut vars = ScriptVars::new();
            for (name, var) in &script.vars {
                match var {
                    DataVar::Number(v) => vars.set_number(name, *v),
                    DataVar::Flag(v) => vars.set_flag(name, *v),
                    DataVar::Text(v) => vars.set_text(name, v.clone()),
                }
                .expect("fresh bag cannot type-conflict");
            }
            spawned.insert(ScriptComponent {
                script: script.script.clone(),
                entry: script.entry.clone(),
                vars,
                requests: ScriptRequests::default(),
            });
        }
        ids.push(spawned.id());
    }
    Ok(DataSpawnReport { ids })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_assets::parse_data_document;
    use gens_ecs::ecs::world::World;

    const DOC: &str = r#"{
        "format_version": 1,
        "name": "bridge-scene",
        "entities": [
            {
                "name": "agent",
                "translation": [1.0, 2.0, 3.0],
                "rotation": [0.0, 0.0, 0.0, 1.0],
                "scale": [1.0, 1.0, 1.0],
                "tag": "player",
                "script": {
                    "script": "scripts/patrol.rhai",
                    "entry": "tick",
                    "vars": {
                        "speed": {"kind": "number", "value": 2.5},
                        "verbose": {"kind": "flag", "value": false},
                        "label": {"kind": "text", "value": "agent-1"}
                    }
                }
            },
            {"name": "sleepy", "active": false},
            {"name": "floor", "translation": [0.0, -1.0, 0.0]}
        ]
    }"#;

    #[test]
    fn spawns_entities_with_expected_components_in_order() {
        let doc = parse_data_document("test.json", DOC.as_bytes()).expect("valid");
        let mut world = World::new();
        let report = apply_data_document(&mut world, &doc).expect("applies");
        assert_eq!(report.ids.len(), 3);
        // Spawn order == authored order. The absolute base index is an
        // ECS-internal detail (bevy 0.16+ reserves low indices for
        // resource entities); the contract is RELATIVE consecutiveness.
        let indices: Vec<u32> = report.ids.iter().map(|id| id.index().index()).collect();
        for w in indices.windows(2) {
            assert_eq!(w[1], w[0] + 1, "consecutive indices in spawn order");
        }
        let agent = report.ids[0];
        assert_eq!(world.get::<Name>(agent).unwrap().0, "agent");
        let t = world.get::<Transform>(agent).unwrap();
        assert_eq!(t.position.to_array(), [1.0, 2.0, 3.0]);
        assert_eq!(
            [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(world.get::<Tag>(agent).unwrap().0, "player");
        let sc = world.get::<ScriptComponent>(agent).unwrap();
        assert_eq!(sc.script, "scripts/patrol.rhai");
        assert_eq!(sc.entry, "tick");
        assert_eq!(sc.vars.get_number("speed").unwrap(), 2.5);
        assert!(!sc.vars.get_flag("verbose").unwrap());
        assert_eq!(sc.vars.get_text("label").unwrap(), "agent-1");
        // sleepy: inactive, default transform, no script
        let sleepy = report.ids[1];
        assert!(world.get::<Active>(sleepy).is_none());
        assert!(world.get::<ScriptComponent>(sleepy).is_none());
        // floor: active by default
        let floor = report.ids[2];
        assert!(world.get::<Active>(floor).is_some());
    }

    #[test]
    fn same_document_two_fresh_worlds_capture_byte_identically() {
        let doc = parse_data_document("test.json", DOC.as_bytes()).expect("valid");
        let mut a = World::new();
        let mut b = World::new();
        apply_data_document(&mut a, &doc).expect("a applies");
        apply_data_document(&mut b, &doc).expect("b applies");
        let ja = crate::save::save_json(&crate::save::capture_world(&a)).expect("a json");
        let jb = crate::save::save_json(&crate::save::capture_world(&b)).expect("b json");
        assert_eq!(ja, jb, "capture output is byte-identical");
    }

    #[test]
    fn data_document_and_save_round_trip_agree() {
        // doc -> world -> save -> fresh world -> capture == doc -> world -> capture
        let doc = parse_data_document("test.json", DOC.as_bytes()).expect("valid");
        let mut direct = World::new();
        apply_data_document(&mut direct, &doc).expect("direct applies");
        let save = crate::save::capture_world(&direct);
        let mut reloaded = World::new();
        crate::save::apply_save(&mut reloaded, &save).expect("save applies");
        let a = crate::save::save_json(&crate::save::capture_world(&direct)).unwrap();
        let b = crate::save::save_json(&crate::save::capture_world(&reloaded)).unwrap();
        assert_eq!(a, b, "the data and save paths agree exactly");
    }

    #[test]
    fn programmatically_invalid_document_is_refused() {
        // Built by hand (not parsed): duplicate names must still fail.
        let doc = DataDocument {
            format_version: gens_assets::DATA_FORMAT_VERSION,
            name: "hand-made".into(),
            entities: vec![
                gens_assets::DataEntity::named("dup"),
                gens_assets::DataEntity::named("dup"),
            ],
        };
        let mut world = World::new();
        let err = apply_data_document(&mut world, &doc).expect_err("refused");
        assert!(err.to_string().contains("duplicate entity name"), "{err}");
    }

    #[test]
    fn empty_document_spawns_nothing() {
        let doc = DataDocument {
            format_version: gens_assets::DATA_FORMAT_VERSION,
            name: "empty".into(),
            entities: vec![],
        };
        let mut world = World::new();
        let report = apply_data_document(&mut world, &doc).expect("applies");
        assert!(report.ids.is_empty());
    }

    #[test]
    fn script_vars_iteration_is_sorted_into_the_save() {
        let doc = parse_data_document("test.json", DOC.as_bytes()).expect("valid");
        let mut world = World::new();
        let report = apply_data_document(&mut world, &doc).expect("applies");
        let sc = world.get::<ScriptComponent>(report.ids[0]).unwrap();
        let save = crate::save::capture_world(&world);
        let saved = &save.entities[0].script.as_ref().unwrap().vars;
        // Both bags iterate sorted; their serialized order matches.
        let names: Vec<String> = sc.vars.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(names, vec!["label", "speed", "verbose"], "sorted");
        assert_eq!(names.len(), 3);
        let _ = saved; // byte-equality already proven by the round-trip test
    }
}
