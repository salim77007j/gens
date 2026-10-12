//! Asset registry: deterministic identity, content hashing, versioning.
//!
//! Every asset is identified by an [`AssetId`] derived from the FNV-1a
//! of its canonical key (its path). Importing the same bytes under the
//! same key twice bumps the entry's `version` when the content changed
//! and leaves it untouched otherwise — so the registry answers "has
//! this asset changed since load?" without filesystem mtime (which is
//! not deterministic across machines).
//!
//! Ordering: the registry is a `BTreeMap` keyed by `AssetId`, so
//! iteration is sorted by id — a save file, a log, or a manifest
//! written from a registry is byte-identical for identical content.

use crate::error::AssetError;
use crate::hash::fnv1a64;

/// Stable asset identifier: FNV-1a 64 of the canonical key.
///
/// Deriving the id from the key (not from insertion order or a counter)
/// makes ids reproducible across machines and process restarts — two
/// registries that loaded the same content agree bit-for-bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId(pub u64);

impl AssetId {
    /// Computes the id for a canonical key.
    pub fn from_key(key: &str) -> AssetId {
        AssetId(fnv1a64(key.as_bytes()))
    }
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "asset:{:016x}", self.0)
    }
}

/// What kind of content an asset holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AssetKind {
    /// glTF 2.0 document (scene, meshes, skins, animations).
    GlTF,
    /// Data-driven scene / prefab document (JSON), Phase 6.
    Data,
    /// Data-driven UI layout (JSON).
    UiLayout,
    /// Script source (text).
    Script,
    /// Opaque bytes (raw content, engine-owned consumers).
    Raw,
}

impl AssetKind {
    /// Human-readable kind name (used in manifests and errors).
    pub fn as_str(self) -> &'static str {
        match self {
            AssetKind::GlTF => "gltf",
            AssetKind::Data => "data",
            AssetKind::UiLayout => "ui_layout",
            AssetKind::Script => "script",
            AssetKind::Raw => "raw",
        }
    }
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One registered asset: identity, content fingerprint and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    /// Canonical key the id was derived from (e.g. "assets/walker.gltf").
    pub key: String,
    /// Kind of content (checked on re-import; a key cannot change kind).
    pub kind: AssetKind,
    /// FNV-1a 64 of the content bytes at the current version.
    pub content_hash: u64,
    /// Monotonic version: 1 on first import, +1 each time the bytes
    /// changed. Unchanged re-imports keep the version.
    pub version: u32,
    /// Content size in bytes.
    pub size: usize,
}

/// Ordered registry of imported assets.
#[derive(Debug, Default)]
pub struct AssetRegistry {
    by_id: std::collections::BTreeMap<AssetId, AssetEntry>,
    by_key: std::collections::BTreeMap<String, AssetId>,
}

impl AssetRegistry {
    /// New empty registry.
    pub fn new() -> AssetRegistry {
        AssetRegistry::default()
    }

    /// Registers (or re-imports) content under `key`.
    ///
    /// First import: version 1. Later imports: version bumps only when
    /// the content hash changed. A kind change for an existing key is
    /// an error (it almost certainly means two producers disagree about
    /// what the path holds).
    pub fn import(
        &mut self,
        key: &str,
        kind: AssetKind,
        bytes: &[u8],
    ) -> Result<(AssetId, &AssetEntry), AssetError> {
        let key = canonicalize_key(key)?;
        if bytes.is_empty() {
            return Err(AssetError::EmptyContent(key));
        }
        if let Some(existing) = self.by_key.get(&key) {
            let entry = self
                .by_id
                .get_mut(existing)
                .expect("key index always points at a live entry");
            if entry.kind != kind {
                return Err(AssetError::KindChanged {
                    key,
                    old: entry.kind,
                    new: kind,
                });
            }
            let id = *existing;
            let hash = fnv1a64(bytes);
            if entry.content_hash != hash {
                entry.version = entry.version.saturating_add(1);
                entry.content_hash = hash;
                entry.size = bytes.len();
            }
            return Ok((id, entry));
        }
        let id = AssetId::from_key(&key);
        // An id collision across different keys is astronomically
        // unlikely (64-bit FNV) but is an error, not silent corruption.
        if self.by_id.contains_key(&id) {
            return Err(AssetError::IdCollision(id));
        }
        let entry = AssetEntry {
            key: key.clone(),
            kind,
            content_hash: fnv1a64(bytes),
            version: 1,
            size: bytes.len(),
        };
        self.by_id.insert(id, entry);
        self.by_key.insert(key, id);
        Ok((id, self.by_id.get(&id).expect("just inserted")))
    }

    /// Registers an asset by reading a file. The key is the path as
    /// given (canonicalized to forward slashes); the kind is inferred
    /// from the extension.
    pub fn import_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(AssetId, AssetKind), AssetError> {
        let path = path.as_ref();
        let key = path.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(path).map_err(|source| AssetError::Io {
            key: key.clone(),
            source,
        })?;
        let kind = kind_from_path(&key);
        let (id, _) = self.import(&key, kind, &bytes)?;
        Ok((id, kind))
    }

    /// Looks up an asset id by canonical key.
    pub fn get(&self, key: &str) -> Option<AssetId> {
        self.by_key.get(key).copied()
    }

    /// Borrows an entry by id.
    pub fn entry(&self, id: AssetId) -> Option<&AssetEntry> {
        self.by_id.get(&id)
    }

    /// Borrows an entry by canonical key.
    pub fn entry_by_key(&self, key: &str) -> Option<&AssetEntry> {
        self.by_key.get(key).and_then(|id| self.by_id.get(id))
    }

    /// Iterates entries ordered by id (deterministic manifest order).
    pub fn iter(&self) -> impl Iterator<Item = (AssetId, &AssetEntry)> {
        self.by_id.iter().map(|(id, e)| (*id, e))
    }

    /// Number of registered assets.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Infers the [`AssetKind`] from a canonical key's extension.
/// Unknown extensions are `Raw` (the caller can always register an
/// explicit kind via [`AssetRegistry::import`]).
fn kind_from_path(key: &str) -> AssetKind {
    let ext = key.rsplit('.').next().unwrap_or("");
    match ext {
        "gltf" | "glb" => AssetKind::GlTF,
        // Phase 6: .json is the data-document extension. The UiLayout
        // kind has no importer yet; it keeps the explicit .uilayout
        // extension so the two never collide.
        "json" => AssetKind::Data,
        "uilayout" => AssetKind::UiLayout,
        "rhai" => AssetKind::Script,
        _ => AssetKind::Raw,
    }
}

/// Normalizes a key: forward slashes, no leading `./`, no trailing slash.
fn canonicalize_key(key: &str) -> Result<String, AssetError> {
    let cleaned = key.replace('\\', "/");
    let cleaned = cleaned.strip_prefix("./").unwrap_or(&cleaned);
    if cleaned.is_empty() || cleaned.ends_with('/') || cleaned.contains("//") {
        return Err(AssetError::InvalidKey(key.to_string()));
    }
    Ok(cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_fnv_of_key() {
        let id = AssetId::from_key("assets/walker.gltf");
        assert_eq!(id.0, crate::hash::fnv1a64(b"assets/walker.gltf"));
        assert_eq!(id.to_string(), format!("asset:{:016x}", id.0));
    }

    #[test]
    fn first_import_is_version_one() {
        let mut reg = AssetRegistry::new();
        let (id, entry) = reg
            .import("a/walker.gltf", AssetKind::GlTF, b"bytes")
            .expect("import");
        assert_eq!(entry.version, 1);
        assert_eq!(entry.kind, AssetKind::GlTF);
        assert_eq!(entry.size, 5);
        assert_eq!(reg.get("a/walker.gltf"), Some(id));
    }

    #[test]
    fn unchanged_reimport_keeps_version() {
        let mut reg = AssetRegistry::new();
        reg.import("x.bin", AssetKind::Raw, b"one").expect("import");
        let (_, entry) = reg
            .import("x.bin", AssetKind::Raw, b"one")
            .expect("reimport");
        assert_eq!(entry.version, 1);
    }

    #[test]
    fn changed_content_bumps_version() {
        let mut reg = AssetRegistry::new();
        reg.import("x.bin", AssetKind::Raw, b"one").expect("import");
        let (_, entry) = reg
            .import("x.bin", AssetKind::Raw, b"two")
            .expect("reimport");
        assert_eq!(entry.version, 2);
        assert_eq!(entry.content_hash, fnv1a64(b"two"));
    }

    #[test]
    fn kind_change_is_rejected() {
        let mut reg = AssetRegistry::new();
        reg.import("x.data", AssetKind::Raw, b"one")
            .expect("import");
        let err = reg
            .import("x.data", AssetKind::Script, b"one")
            .expect_err("kind change");
        assert!(matches!(err, AssetError::KindChanged { .. }));
    }

    #[test]
    fn dot_slash_prefix_canonicalizes_then_empty_rejected() {
        let mut reg = AssetRegistry::new();
        // "./x.bin" canonicalizes to "x.bin"; the failure is the EMPTY
        // content, proving the key was accepted before the content check.
        let err = reg
            .import("./x.bin", AssetKind::Raw, b"")
            .expect_err("empty");
        assert!(matches!(err, AssetError::EmptyContent(ref k) if k == "x.bin"));
    }

    #[test]
    fn invalid_keys_rejected() {
        let mut reg = AssetRegistry::new();
        for bad in ["", "a//b.bin", "dir/"] {
            let err = reg.import(bad, AssetKind::Raw, b"x").expect_err(bad);
            assert!(matches!(err, AssetError::InvalidKey(_)), "{bad}");
        }
    }

    #[test]
    fn windows_separators_canonicalize() {
        let mut reg = AssetRegistry::new();
        let (id, _) = reg
            .import("a\\b\\c.rhai", AssetKind::Script, b"fn f(){}")
            .expect("import");
        assert_eq!(reg.get("a/b/c.rhai"), Some(id));
    }

    #[test]
    fn empty_content_rejected() {
        let mut reg = AssetRegistry::new();
        let err = reg.import("x.bin", AssetKind::Raw, b"").expect_err("empty");
        assert!(matches!(err, AssetError::EmptyContent(_)));
    }

    #[test]
    fn iteration_is_id_ordered() {
        let mut reg = AssetRegistry::new();
        reg.import("b.bin", AssetKind::Raw, b"1").expect("b");
        reg.import("a.bin", AssetKind::Raw, b"2").expect("a");
        reg.import("c.bin", AssetKind::Raw, b"3").expect("c");
        let keys: Vec<String> = reg.iter().map(|(_, e)| e.key.clone()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        // Sorted by id, not by key — but stable across identical content.
        assert_eq!(keys.len(), 3);
        assert_eq!(keys, sorted);
    }

    #[test]
    fn kind_from_path_inference() {
        assert_eq!(kind_from_path("a/w.gltf"), AssetKind::GlTF);
        assert_eq!(kind_from_path("a/scene.json"), AssetKind::Data);
        assert_eq!(kind_from_path("a/hud.uilayout"), AssetKind::UiLayout);
        assert_eq!(kind_from_path("a/s.rhai"), AssetKind::Script);
        assert_eq!(kind_from_path("a/r.xyz"), AssetKind::Raw);
    }
}
