//! The `GENSPKG1` package container: build + verify (P8-5).
//!
//! Layout (byte-exact, the save format's framing discipline applied to a
//! game export):
//!
//! ```text
//! GENSPKG1\n
//! {"format_version":1,"name":..,"scene_key":..,"engine_version":..,"files":[{"key":..,"kind":..,"len":..,"hash":..},..]}\n
//! GENSDATA <total bytes of all blobs>\n
//! <blob for files[0]><blob for files[1]>...
//! GENSHASH <fnv1a64 of every byte above>\n
//! ```
//!
//! Loud errors everywhere ([`PackageError`]): bad magic, unsupported
//! version, malformed manifest, data-length mismatch, trailer hash
//! mismatch, per-file hash mismatch. A package that survives
//! [`verify_package`] has every file byte-exact against its recorded
//! hash — content integrity is not optional.

use gens_assets::{AssetKind, DataDocument, fnv1a64};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Magic line.
pub const PACKAGE_MAGIC: &str = "GENSPKG1";
/// Container format version.
pub const PACKAGE_FORMAT_VERSION: u32 = 1;

/// One file entry in the manifest (order = blob order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    /// Registry key (`scenes/main.json`, `scripts/patrol.rhai`, ...).
    pub key: String,
    /// [`AssetKind::as_str`] of the content.
    pub kind: String,
    /// Blob length in bytes.
    pub len: u64,
    /// FNV-1a 64 of the blob bytes.
    pub hash: u64,
}

/// The manifest (JSON line 2). Field order is the serialization order —
/// canonical, byte-stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageManifest {
    /// Container format version (must equal [`PACKAGE_FORMAT_VERSION`]).
    pub format_version: u32,
    /// Project name.
    pub name: String,
    /// Registry key of the scene data document.
    pub scene_key: String,
    /// Engine version that built the package (informational; player
    /// reports it, nothing gates on it yet).
    pub engine_version: String,
    /// Files in blob order.
    pub files: Vec<PackageFile>,
}

/// A verified package: manifest + blobs + the parsed scene document.
#[derive(Debug, Clone)]
pub struct VerifiedPackage {
    /// Parsed manifest.
    pub manifest: PackageManifest,
    /// Blob bytes by key (BTreeMap: iteration is key-sorted).
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// The scene document (already parsed from the scene_key blob).
    pub scene: DataDocument,
}

/// Package errors — loud, specific, unambiguous.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// The magic line is missing or wrong.
    #[error("not a GENSPKG1 package (bad magic line)")]
    BadMagic,
    /// The manifest's format_version is newer than this build knows.
    #[error("package format_version {found} unsupported (this build knows {known})")]
    UnsupportedVersion {
        /// Version found in the manifest.
        found: u32,
        /// Version this build supports.
        known: u32,
    },
    /// The manifest line is not valid JSON or fails validation.
    #[error("manifest malformed: {0}")]
    MalformedManifest(String),
    /// The GENSDATA line is missing or its length disagrees with the file
    /// table.
    #[error("data section malformed: {0}")]
    MalformedData(String),
    /// The data section is shorter than the declared total (truncated).
    #[error("package truncated: declared {declared} data bytes, found {found}")]
    Truncated {
        /// Declared total in the GENSDATA line.
        declared: u64,
        /// Bytes actually present.
        found: u64,
    },
    /// The GENSHASH trailer is missing or does not match the content.
    #[error(
        "package hash mismatch: trailer says {expected:016x}, content hashes to {computed:016x}"
    )]
    Corrupt {
        /// Hash recorded in the trailer.
        expected: u64,
        /// Hash computed over the received bytes.
        computed: u64,
    },
    /// A file's bytes do not match its recorded hash.
    #[error("file '{key}' corrupt: recorded {expected:016x}, computed {computed:016x}")]
    FileHashMismatch {
        /// Offending file key.
        key: String,
        /// Hash recorded in the manifest.
        expected: u64,
        /// Hash computed over the received bytes.
        computed: u64,
    },
    /// A file's length disagrees with its recorded len.
    #[error("file '{key}' length mismatch: recorded {expected}, found {found}")]
    FileLengthMismatch {
        /// Offending file key.
        key: String,
        /// Length recorded in the manifest.
        expected: u64,
        /// Bytes actually present.
        found: u64,
    },
    /// The scene blob is not a valid data document.
    #[error("scene is not a valid data document: {0}")]
    BadScene(#[source] gens_assets::AssetError),
    /// The build input is invalid (duplicate/empty key, empty content...).
    #[error("build input invalid: {0}")]
    BadInput(String),
}

/// Parses an [`AssetKind`] from its manifest string.
pub fn parse_kind(s: &str) -> Option<AssetKind> {
    match s {
        "gltf" => Some(AssetKind::GlTF),
        "data" => Some(AssetKind::Data),
        "ui_layout" => Some(AssetKind::UiLayout),
        "script" => Some(AssetKind::Script),
        "raw" => Some(AssetKind::Raw),
        _ => None,
    }
}

/// Builds a package from parts.
///
/// `scene_key`/`scene_bytes` must parse as a [`DataDocument`] (validated
/// HERE, at build time — an invalid scene never becomes a package).
/// File keys must be unique, non-empty, canonical (no leading/trailing
/// slash, no `//`), and must not collide with the scene key.
pub fn build_package(
    name: &str,
    scene_key: &str,
    scene_bytes: &[u8],
    files: Vec<(String, AssetKind, Vec<u8>)>,
) -> Result<Vec<u8>, PackageError> {
    if name.trim().is_empty() {
        return Err(PackageError::BadInput("project name is empty".into()));
    }
    gens_assets::canonicalize_key(scene_key)
        .map_err(|e| PackageError::BadInput(format!("scene key invalid: {e}")))?;

    // The scene must be a valid data document at build time.
    gens_assets::parse_data_document(scene_key, scene_bytes).map_err(PackageError::BadScene)?;

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut entries = Vec::with_capacity(files.len() + 1);
    let mut blobs: Vec<u8> = Vec::new();

    // Scene first, then files in the given order (deterministic: the
    // caller owns file order; the CLI sorts).
    entries.push(PackageFile {
        key: scene_key.to_string(),
        kind: AssetKind::Data.as_str().to_string(),
        len: scene_bytes.len() as u64,
        hash: fnv1a64(scene_bytes),
    });
    blobs.extend_from_slice(scene_bytes);

    for (key, kind, bytes) in files {
        gens_assets::canonicalize_key(&key)
            .map_err(|e| PackageError::BadInput(format!("file key '{key}' invalid: {e}")))?;
        if bytes.is_empty() {
            return Err(PackageError::BadInput(format!("file '{key}' is empty")));
        }
        if key == scene_key {
            return Err(PackageError::BadInput(format!(
                "file key '{key}' collides with the scene key"
            )));
        }
        if !seen.insert(key.clone()) {
            return Err(PackageError::BadInput(format!(
                "duplicate file key '{key}'"
            )));
        }
        entries.push(PackageFile {
            key: key.clone(),
            kind: kind.as_str().to_string(),
            len: bytes.len() as u64,
            hash: fnv1a64(&bytes),
        });
        blobs.extend_from_slice(&bytes);
    }

    let manifest = PackageManifest {
        format_version: PACKAGE_FORMAT_VERSION,
        name: name.to_string(),
        scene_key: scene_key.to_string(),
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        files: entries,
    };
    let manifest_json = serde_json::to_string(&manifest)
        .map_err(|e| PackageError::BadInput(format!("manifest serialize: {e}")))?;

    let mut out = Vec::new();
    out.extend_from_slice(PACKAGE_MAGIC.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(manifest_json.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(format!("GENSDATA {}\n", blobs.len()).as_bytes());
    out.extend_from_slice(&blobs);
    out.extend_from_slice(format!("GENSHASH {:016x}\n", fnv1a64(&out)).as_bytes());
    Ok(out)
}

/// Splits `data` at the first `\n`, returning the line (without `\n`) and
/// the remainder. Errors when no newline exists.
fn take_line<'a>(data: &'a [u8], what: &'static str) -> Result<(&'a [u8], &'a [u8]), PackageError> {
    let pos = data
        .iter()
        .position(|&b| b == b'\n')
        .ok_or(PackageError::Truncated {
            declared: 0,
            found: data.len() as u64,
        })
        .map_err(|_| PackageError::MalformedData(what.to_string()))?;
    Ok((&data[..pos], &data[pos + 1..]))
}

/// Verifies a package: magic, manifest, declared lengths, trailer hash
/// and per-file hashes. Returns the parsed manifest, blobs and scene.
pub fn verify_package(bytes: &[u8]) -> Result<VerifiedPackage, PackageError> {
    let (magic, rest) = take_line(bytes, "magic line")?;
    if magic != PACKAGE_MAGIC.as_bytes() {
        return Err(PackageError::BadMagic);
    }
    let (manifest_line, rest) = take_line(rest, "manifest line")?;
    let manifest: PackageManifest = serde_json::from_slice(manifest_line)
        .map_err(|e| PackageError::MalformedManifest(e.to_string()))?;
    if manifest.format_version != PACKAGE_FORMAT_VERSION {
        return Err(PackageError::UnsupportedVersion {
            found: manifest.format_version,
            known: PACKAGE_FORMAT_VERSION,
        });
    }
    if manifest.files.is_empty() {
        return Err(PackageError::MalformedManifest(
            "file table is empty".into(),
        ));
    }
    if !manifest.files.iter().any(|f| f.key == manifest.scene_key) {
        return Err(PackageError::MalformedManifest(format!(
            "scene key '{}' missing from the file table",
            manifest.scene_key
        )));
    }

    let (data_line, rest) = take_line(rest, "GENSDATA line")?;
    let data_line = std::str::from_utf8(data_line)
        .map_err(|_| PackageError::MalformedData("GENSDATA line is not UTF-8".into()))?;
    let total: u64 = data_line
        .strip_prefix("GENSDATA ")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| PackageError::MalformedData(data_line.to_string()))?;

    if (rest.len() as u64) < total {
        return Err(PackageError::Truncated {
            declared: total,
            found: rest.len() as u64,
        });
    }
    let blobs_region = &rest[..total as usize];
    let trailer_and_after = &rest[total as usize..];

    // Trailer: GENSHASH <hex>\n and nothing after.
    let (trailer_line, after) = take_line(trailer_and_after, "trailer line")?;
    if !after.is_empty() {
        return Err(PackageError::MalformedData(
            "trailing bytes after the trailer".into(),
        ));
    }
    let trailer = std::str::from_utf8(trailer_line)
        .map_err(|_| PackageError::MalformedData("trailer line is not UTF-8".into()))?;
    let expected = trailer
        .strip_prefix("GENSHASH ")
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .ok_or_else(|| PackageError::MalformedData(trailer.to_string()))?;
    let header_and_data_len = bytes.len() - trailer_and_after.len();
    let computed = fnv1a64(&bytes[..header_and_data_len]);
    if expected != computed {
        return Err(PackageError::Corrupt { expected, computed });
    }

    // Split blobs by recorded lengths, hash-check each.
    let mut blobs = BTreeMap::new();
    let mut cursor = blobs_region;
    for file in &manifest.files {
        let len = file.len as usize;
        if cursor.len() < len {
            return Err(PackageError::FileLengthMismatch {
                key: file.key.clone(),
                expected: file.len,
                found: cursor.len() as u64,
            });
        }
        let (blob, rest) = cursor.split_at(len);
        let computed = fnv1a64(blob);
        if computed != file.hash {
            return Err(PackageError::FileHashMismatch {
                key: file.key.clone(),
                expected: file.hash,
                computed,
            });
        }
        blobs.insert(file.key.clone(), blob.to_vec());
        cursor = rest;
    }
    if !cursor.is_empty() {
        return Err(PackageError::MalformedData(format!(
            "{} trailing data bytes after the last file",
            cursor.len()
        )));
    }

    let scene_bytes = blobs
        .get(&manifest.scene_key)
        .expect("scene presence checked above");
    let scene = gens_assets::parse_data_document(&manifest.scene_key, scene_bytes)
        .map_err(PackageError::BadScene)?;

    Ok(VerifiedPackage {
        manifest,
        blobs,
        scene,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"{
        "format_version": 1,
        "name": "tiny-scene",
        "entities": [
            {"name": "box", "translation": [0.0, 1.0, 0.0], "rotation": [0.0, 0.0, 0.0, 1.0], "scale": [1.0, 1.0, 1.0]}
        ]
    }"#;

    fn sample_files() -> Vec<(String, AssetKind, Vec<u8>)> {
        vec![(
            "scripts/idle.rhai".into(),
            AssetKind::Script,
            b"fn update() {}\n".to_vec(),
        )]
    }

    #[test]
    fn build_verify_roundtrip_is_exact() {
        let pkg = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build");
        let verified = verify_package(&pkg).expect("verify");
        assert_eq!(verified.manifest.name, "tiny");
        assert_eq!(verified.manifest.scene_key, "scenes/tiny.json");
        assert_eq!(verified.manifest.files.len(), 2);
        assert_eq!(verified.manifest.files[0].kind, "data");
        assert_eq!(verified.manifest.files[1].kind, "script");
        assert_eq!(verified.scene.name, "tiny-scene");
        assert_eq!(verified.blobs["scripts/idle.rhai"], b"fn update() {}\n");
        // Byte-level stability: same inputs, same package bytes.
        let pkg2 = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build 2");
        assert_eq!(pkg, pkg2, "build must be byte-deterministic");
    }

    #[test]
    fn bad_magic_is_loud() {
        let pkg =
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), vec![]).expect("build");
        let mut corrupted = pkg.clone();
        corrupted[0] = b'X';
        assert!(matches!(
            verify_package(&corrupted),
            Err(PackageError::BadMagic)
        ));
    }

    #[test]
    fn corrupt_content_is_caught_by_the_trailer_hash() {
        let pkg = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build");
        // Flip one byte INSIDE the blob region.
        let mut corrupted = pkg.clone();
        let mid = corrupted.len() * 2 / 3;
        corrupted[mid] ^= 0xFF;
        assert!(matches!(
            verify_package(&corrupted),
            Err(PackageError::Corrupt { .. })
        ));
    }

    #[test]
    fn corrupt_manifest_is_caught_loudly() {
        let pkg = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build");
        let mut corrupted = pkg.clone();
        // Flip a byte in the manifest line (past the magic). Verify order:
        // manifest parse runs BEFORE the trailer hash, so this surfaces as
        // a manifest/-version error — loud either way; the trailer gate
        // itself is covered by corrupt_content above.
        let flip = PACKAGE_MAGIC.len() + 1 + 5;
        corrupted[flip] ^= 0x01;
        assert!(matches!(
            verify_package(&corrupted),
            Err(PackageError::MalformedManifest(_)
                | PackageError::Corrupt { .. }
                | PackageError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn truncated_package_is_caught() {
        let pkg = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build");
        let truncated = &pkg[..pkg.len() - 30];
        assert!(matches!(
            verify_package(truncated),
            Err(PackageError::Truncated { .. }) | Err(PackageError::MalformedData(_))
        ));
    }

    #[test]
    fn unsupported_future_version_is_rejected() {
        let pkg =
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), vec![]).expect("build");
        // Rewrite the manifest's format_version and FIX the trailer so the
        // rejection is specifically the version gate, not a hash failure.
        let text = String::from_utf8(pkg).expect("package is text-only in this fixture");
        let patched = text.replace("\"format_version\":1", "\"format_version\":99");
        let rebuilt = rebuild_trailer(patched);
        assert!(matches!(
            verify_package(&rebuilt),
            Err(PackageError::UnsupportedVersion {
                found: 99,
                known: 1
            })
        ));
    }

    /// Recomputes the GENSHASH trailer after a content edit (test helper:
    /// isolates the manifest gate from the integrity gate).
    fn rebuild_trailer(mut text: String) -> Vec<u8> {
        let pos = text.rfind("GENSHASH ").expect("trailer present");
        text.truncate(pos);
        let hash = fnv1a64(text.as_bytes());
        text.push_str(&format!("GENSHASH {hash:016x}\n"));
        text.into_bytes()
    }

    #[test]
    fn invalid_scene_never_becomes_a_package() {
        let bad = b"{ not json }".to_vec();
        let err = build_package("tiny", "scenes/bad.json", &bad, vec![]).expect_err("must fail");
        assert!(matches!(err, PackageError::BadScene(_)));
    }

    #[test]
    fn duplicate_keys_and_collisions_are_rejected() {
        let dup = vec![
            (
                "scripts/a.rhai".to_string(),
                AssetKind::Script,
                b"fn f() {}".to_vec(),
            ),
            (
                "scripts/a.rhai".to_string(),
                AssetKind::Script,
                b"fn g() {}".to_vec(),
            ),
        ];
        let err =
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), dup).expect_err("dup");
        assert!(matches!(err, PackageError::BadInput(_)));

        let collision = vec![(
            "scenes/tiny.json".to_string(),
            AssetKind::Raw,
            b"sneaky".to_vec(),
        )];
        let err = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), collision)
            .expect_err("collision");
        assert!(matches!(err, PackageError::BadInput(_)));
    }

    #[test]
    fn empty_and_noncanonical_keys_are_rejected() {
        let empty_key = vec![("".to_string(), AssetKind::Raw, b"x".to_vec())];
        assert!(matches!(
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), empty_key),
            Err(PackageError::BadInput(_))
        ));
        // The SHARED registry key rule rejects double slashes and trailing
        // slashes (a leading '/' is canonicalized through unchanged — the
        // package builder deliberately uses the same rule, one source of
        // truth, so "rejected at build" == "rejected at registry").
        let double_slash = vec![("assets//x.png".to_string(), AssetKind::Raw, b"x".to_vec())];
        assert!(matches!(
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), double_slash),
            Err(PackageError::BadInput(_))
        ));
        let trailing = vec![("assets/".to_string(), AssetKind::Raw, b"x".to_vec())];
        assert!(matches!(
            build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), trailing),
            Err(PackageError::BadInput(_))
        ));
    }

    #[test]
    fn parse_kind_covers_every_variant() {
        for (s, kind) in [
            ("gltf", AssetKind::GlTF),
            ("data", AssetKind::Data),
            ("ui_layout", AssetKind::UiLayout),
            ("script", AssetKind::Script),
            ("raw", AssetKind::Raw),
        ] {
            assert_eq!(parse_kind(s), Some(kind));
        }
        assert_eq!(parse_kind("nonsense"), None);
    }

    #[test]
    fn file_hash_mismatch_is_caught_per_file() {
        // Build a valid package, then corrupt ONE blob byte and REPAIR the
        // trailer — the per-file hash gate must catch it (belt to the
        // trailer's suspenders).
        let pkg = build_package("tiny", "scenes/tiny.json", SCENE.as_bytes(), sample_files())
            .expect("build");
        let text = String::from_utf8(pkg).expect("text-only fixture");
        // The script blob is at the very end before the trailer: flip its
        // last content byte.
        let trailer_at = text.rfind("GENSHASH ").expect("trailer");
        let mut bytes = text.into_bytes();
        bytes[trailer_at - 2] ^= 0x01; // inside the script blob ("\n" before trailer)
        let rebuilt = rebuild_trailer(String::from_utf8(bytes).expect("still text"));
        assert!(matches!(
            verify_package(&rebuilt),
            Err(PackageError::FileHashMismatch { .. })
        ));
    }
}
