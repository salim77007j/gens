//! Versioned world save/load (Phase 5).
//!
//! A save file is a **self-contained, versioned document** — mirror
//! types, not live ECS types — so the on-disk format can evolve
//! independently of the engine's internals:
//!
//! ```text
//! GENSSAVE1\n              (magic + format version)
//! {"format_version":1, ...}\n
//! GENSHASH <fnv1a64(body)>\n
//! ```
//!
//! Determinism contract: saving the same world state twice produces
//! byte-identical files (entities sorted by index, struct field order
//! fixed, `ScriptVars`/`ScriptRequests` are BTreeMaps). Loading into a
//! FRESH world and continuing the simulation is bit-identical to an
//! uninterrupted run — the property the P5-6 example pins.
//!
//! Scope (ADR-0021): the save covers the engine's standard component
//! vocabulary — name, active, tag, transform, script (vars + requests).
//! Parent/child links, game-defined components and heavyweight
//! resources (animation controllers holding `Arc<Clip>` libraries) are
//! NOT saved; game state that must survive a save/load travels through
//! `ScriptComponent::vars`, which IS covered. A reflection/registry
//! layer for arbitrary components is future work.

use gens_assets::fnv1a64;
use gens_ecs::{Active, Name, Tag, Transform};
use gens_math::{Quat, Vec3};
use gens_scripting::{ScriptComponent, ScriptRequests, ScriptVars};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The magic header line (includes the format version).
pub const SAVE_MAGIC: &str = "GENSSAVE1";
/// Current [`SaveWorld::format_version`].
pub const FORMAT_VERSION: u32 = 1;

/// Save/load errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaveError {
    /// The file's magic line does not match [`SAVE_MAGIC`].
    #[error("not a gens save file (expected {SAVE_MAGIC:?}, got {found:?})")]
    BadMagic {
        /// What the file actually started with.
        found: String,
    },
    /// The body's hash does not match the trailer.
    #[error("save file corrupt: hash mismatch (recorded {recorded:#x}, computed {computed:#x})")]
    HashMismatch {
        /// Hash recorded in the trailer.
        recorded: u64,
        /// Hash recomputed over the body.
        computed: u64,
    },
    /// The document parsed but its `format_version` is unknown.
    #[error("unsupported save format version {found} (this engine writes {FORMAT_VERSION})")]
    BadVersion {
        /// The version the file carries.
        found: u32,
    },
    /// JSON parse failure (with location context from serde).
    #[error("save file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Filesystem failure at the save boundary.
    #[error("save I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias for save results.
pub type SaveResult<T> = Result<T, SaveError>;

/// One saved entity, in the file's canonical order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedEntity {
    /// The entity's index (its position in the file == spawn order).
    pub index: u32,
    /// Human-readable name, when present.
    pub name: Option<String>,
    /// Local transform, when present.
    pub transform: Option<SavedTransform>,
    /// The `Active` marker, when present.
    pub active: bool,
    /// The free-form tag, when present.
    pub tag: Option<String>,
    /// Script state, when the entity is scripted.
    pub script: Option<SavedScript>,
}

/// Position/rotation/scale as plain numbers (decoupled from glam's
/// internal representation).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedTransform {
    /// Local position.
    pub position: [f32; 3],
    /// Local rotation (x, y, z, w).
    pub rotation: [f32; 4],
    /// Local scale.
    pub scale: [f32; 3],
}

impl SavedTransform {
    fn from_transform(t: &Transform) -> SavedTransform {
        SavedTransform {
            position: t.position.to_array(),
            rotation: [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
            scale: t.scale.to_array(),
        }
    }

    fn to_transform(self) -> Transform {
        Transform {
            position: Vec3::from_array(self.position),
            rotation: Quat::from_xyzw(
                self.rotation[0],
                self.rotation[1],
                self.rotation[2],
                self.rotation[3],
            ),
            scale: Vec3::from_array(self.scale),
        }
    }
}

/// Script identity + state as saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedScript {
    /// Script name in the host.
    pub script: String,
    /// Entry function name.
    pub entry: String,
    /// The variable bag.
    pub vars: ScriptVars,
    /// Last tick's animation requests.
    pub requests: ScriptRequests,
}

/// The whole save document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveWorld {
    /// Writer's format version ([`FORMAT_VERSION`]).
    pub format_version: u32,
    /// All entities, sorted by index (the canonical order).
    pub entities: Vec<SavedEntity>,
}

/// Captures the saveable state of `world`.
///
/// Entities are sorted by entity index; entities with none of the
/// saveable components are skipped (nothing would be restored).
pub fn capture_world(world: &gens_ecs::ecs::world::World) -> SaveWorld {
    // Archetype-order iteration is NOT deterministic; the sort by
    // entity index below is what makes the document canonical.
    let mut entities: Vec<SavedEntity> = world
        .iter_entities()
        .filter_map(|entity_ref| {
            let entity = entity_ref.id();
            let name = world.get::<Name>(entity);
            let transform = world.get::<Transform>(entity);
            let active = world.get::<Active>(entity);
            let tag = world.get::<Tag>(entity);
            let script = world.get::<ScriptComponent>(entity);
            if name.is_none()
                && transform.is_none()
                && active.is_none()
                && tag.is_none()
                && script.is_none()
            {
                return None; // nothing saveable
            }
            Some(SavedEntity {
                index: entity.index().index(),
                name: name.map(|n| n.0.clone()),
                transform: transform.map(SavedTransform::from_transform),
                active: active.is_some(),
                tag: tag.map(|t| t.0.clone()),
                script: script.map(|sc| SavedScript {
                    script: sc.script.clone(),
                    entry: sc.entry.clone(),
                    vars: sc.vars.clone(),
                    requests: sc.requests.clone(),
                }),
            })
        })
        .collect();
    entities.sort_by_key(|e| e.index);
    SaveWorld {
        format_version: FORMAT_VERSION,
        entities,
    }
}

/// Spawns `save`'s entities into `world` (which should be FRESH: save
/// order assumes the allocator hands out indices sequentially).
///
/// After spawning, [`EngineSchedules::run_frame`] recomputes
/// `WorldTransform` (never saved — always derived).
///
/// # Errors
/// [`SaveError::BadVersion`] when the document is from another format
/// version.
pub fn apply_save(world: &mut gens_ecs::ecs::world::World, save: &SaveWorld) -> SaveResult<()> {
    if save.format_version != FORMAT_VERSION {
        return Err(SaveError::BadVersion {
            found: save.format_version,
        });
    }
    for entity in &save.entities {
        let mut spawned = world.spawn_empty();
        if let Some(name) = &entity.name {
            spawned.insert(Name::new(name.clone()));
        }
        if let Some(transform) = &entity.transform {
            spawned.insert(transform.to_transform());
        }
        if entity.active {
            spawned.insert(Active);
        }
        if let Some(tag) = &entity.tag {
            spawned.insert(Tag::new(tag.clone()));
        }
        if let Some(script) = &entity.script {
            spawned.insert(ScriptComponent {
                script: script.script.clone(),
                entry: script.entry.clone(),
                vars: script.vars.clone(),
                requests: script.requests.clone(),
            });
        }
        let _ = spawned.id();
        let _ = entity.index; // spawn order == index order; see module docs
    }
    Ok(())
}

/// Serializes a save document to the canonical JSON body.
pub fn save_json(save: &SaveWorld) -> SaveResult<String> {
    Ok(serde_json::to_string(save)?)
}

/// Parses and version-checks a JSON body.
pub fn load_json(json: &str) -> SaveResult<SaveWorld> {
    let save: SaveWorld = serde_json::from_str(json)?;
    if save.format_version != FORMAT_VERSION {
        return Err(SaveError::BadVersion {
            found: save.format_version,
        });
    }
    Ok(save)
}

/// Writes the full save file (magic + body + hash trailer) atomically:
/// the payload lands at `<path>.tmp` and is renamed over `path` — a
/// crash mid-write leaves the previous save intact.
///
/// Returns the bytes written.
pub fn save_world_to_file(
    world: &gens_ecs::ecs::world::World,
    path: impl AsRef<Path>,
) -> SaveResult<usize> {
    let save = capture_world(world);
    let body = save_json(&save)?;
    let file = format_file(&body);
    atomic_write(path, file.as_bytes())?;
    Ok(file.len())
}

/// Reads and fully validates a save file (magic, hash, version).
pub fn load_world_from_file(path: impl AsRef<Path>) -> SaveResult<SaveWorld> {
    let raw = std::fs::read_to_string(path)?;
    let body = parse_file(&raw)?;
    load_json(&body)
}

/// Formats the complete file text around a JSON body.
fn format_file(body: &str) -> String {
    format!(
        "{SAVE_MAGIC}\n{body}\nGENSHASH {:016x}\n",
        fnv1a64(body.as_bytes())
    )
}

/// Splits and verifies a file's framing; returns the JSON body.
fn parse_file(raw: &str) -> SaveResult<String> {
    let mut lines = raw.lines();
    let magic = lines.next().unwrap_or("");
    if magic != SAVE_MAGIC {
        return Err(SaveError::BadMagic {
            found: magic.to_string(),
        });
    }
    let body = lines
        .next()
        .ok_or_else(|| SaveError::BadMagic {
            found: "(empty)".to_string(),
        })?
        .to_string();
    let trailer = lines.next().unwrap_or("");
    let recorded = trailer
        .strip_prefix("GENSHASH ")
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .ok_or_else(|| SaveError::HashMismatch {
            recorded: 0,
            computed: fnv1a64(body.as_bytes()),
        })?;
    let computed = fnv1a64(body.as_bytes());
    if recorded != computed {
        return Err(SaveError::HashMismatch { recorded, computed });
    }
    Ok(body)
}

/// Atomic file write: temp file + rename.
fn atomic_write(path: impl AsRef<Path>, bytes: &[u8]) -> SaveResult<()> {
    let path = path.as_ref();
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Convenience: capture + serialize in one call (used by tests and the
/// example's evidence).
pub fn save_world_json(world: &gens_ecs::ecs::world::World) -> SaveResult<String> {
    save_json(&capture_world(world))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_ecs::ecs::world::World;
    use gens_scripting::ScriptComponent;

    fn scripted_entity(
        world: &mut World,
        name: &str,
        x: f32,
        script: &str,
    ) -> gens_ecs::ecs::prelude::Entity {
        world
            .spawn((
                Name::new(name),
                Transform::from_position(Vec3::new(x, 0.0, 0.0)),
                Active,
                Tag::new("agent"),
                {
                    let mut sc = ScriptComponent::new(script);
                    sc.vars.set_number("phase", 0.5).expect("set");
                    sc.vars.set_flag("moving", true).expect("set");
                    sc.requests.play = Some("walk".into());
                    sc
                },
            ))
            .id()
    }

    #[test]
    fn save_is_byte_deterministic() {
        let build = || {
            let mut world = World::new();
            scripted_entity(&mut world, "a", 1.0, "patrol");
            scripted_entity(&mut world, "b", 2.0, "patrol");
            world
        };
        let j1 = save_world_json(&build()).expect("save");
        let j2 = save_world_json(&build()).expect("save");
        assert_eq!(j1, j2);
    }

    #[test]
    fn roundtrip_restores_all_components() {
        let mut world = World::new();
        scripted_entity(&mut world, "hero", 3.25, "patrol");
        let save = capture_world(&world);
        assert_eq!(save.entities.len(), 1);
        let json = save_json(&save).expect("serialize");

        let loaded = load_json(&json).expect("parse");
        assert_eq!(loaded.format_version, 1);
        let mut fresh = World::new();
        apply_save(&mut fresh, &loaded).expect("apply");
        let save2 = capture_world(&fresh);
        assert_eq!(save, save2);

        // Component-level spot checks.
        let mut q = fresh.query::<(&Name, &Transform, &Active, &Tag, &ScriptComponent)>();
        let (name, transform, _active, tag, sc) = q.iter(&fresh).next().expect("one entity");
        assert_eq!(name.as_str(), "hero");
        assert_eq!(transform.position, Vec3::new(3.25, 0.0, 0.0));
        assert_eq!(tag.0, "agent");
        assert_eq!(sc.script, "patrol");
        assert_eq!(sc.vars.get_number("phase").expect("var"), 0.5);
        assert!(sc.vars.get_flag("moving").expect("var"));
        assert_eq!(sc.requests.play.as_deref(), Some("walk"));
    }

    #[test]
    fn entity_order_is_index_sorted() {
        let mut world = World::new();
        let _b = scripted_entity(&mut world, "b", 2.0, "patrol");
        let _a = scripted_entity(&mut world, "a", 1.0, "patrol");
        let save = capture_world(&world);
        let indices: Vec<u32> = save.entities.iter().map(|e| e.index).collect();
        let mut sorted = indices.clone();
        sorted.sort_unstable();
        assert_eq!(indices, sorted);
    }

    #[test]
    fn file_framing_and_hash_verification() {
        let mut world = World::new();
        scripted_entity(&mut world, "a", 1.0, "patrol");
        let save = capture_world(&world);
        let body = save_json(&save).expect("body");
        let file = format_file(&body);
        assert!(file.starts_with("GENSSAVE1\n"));
        assert!(file.contains("GENSHASH "));
        assert_eq!(parse_file(&file).expect("parse"), body);
    }

    #[test]
    fn corrupt_body_is_rejected() {
        let mut world = World::new();
        scripted_entity(&mut world, "a", 1.0, "patrol");
        let body = save_json(&capture_world(&world)).expect("body");
        let mut file = format_file(&body);
        // Flip a digit inside the body.
        let mid = file.len() / 2;
        file.replace_range(mid..mid + 1, "9");
        let err = parse_file(&file).expect_err("must fail hash");
        assert!(matches!(err, SaveError::HashMismatch { .. }));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let err = parse_file("NOTAGENS\n{}\nGENSHASH 00\n").expect_err("magic");
        assert!(matches!(err, SaveError::BadMagic { .. }));
    }

    #[test]
    fn future_version_is_rejected() {
        let save = SaveWorld {
            format_version: 99,
            entities: vec![],
        };
        let body = serde_json::to_string(&save).expect("body");
        let err = load_json(&body).expect_err("version");
        assert!(matches!(err, SaveError::BadVersion { found: 99 }));
    }

    #[test]
    fn atomic_write_leaves_no_tmp_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("gens_save_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("world.save");
        let mut world = World::new();
        scripted_entity(&mut world, "a", 1.0, "patrol");
        save_world_to_file(&world, &path).expect("write");
        let loaded = load_world_from_file(&path).expect("read");
        assert_eq!(loaded, capture_world(&world));
        // The tmp file was renamed away.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp."))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn world_transform_is_never_saved() {
        // WorldTransform is derived; a captured save never contains it.
        let mut world = World::new();
        let e = scripted_entity(&mut world, "a", 1.0, "patrol");
        let mut schedules = gens_ecs::EngineSchedules::default();
        schedules.run_frame(&mut world);
        assert!(world.get::<gens_ecs::WorldTransform>(e).is_some());
        let save = capture_world(&world);
        let json = save_json(&save).expect("body");
        assert!(!json.contains("WorldTransform"));
        assert!(!json.contains("world_transform"));
    }
}
