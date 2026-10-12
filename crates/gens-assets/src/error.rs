//! Asset pipeline errors.

/// Errors raised by the asset registry and importers.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AssetError {
    /// A key is empty, ends in `/`, or contains `//`.
    #[error("invalid asset key: {0:?}")]
    InvalidKey(String),
    /// Empty content cannot be registered (an empty file is almost
    /// always a failed export; registering it would hash the same
    /// bytes for every empty asset).
    #[error("empty content for asset {0:?}")]
    EmptyContent(String),
    /// An existing key was re-imported as a different kind.
    #[error("asset {key:?} was imported as {old} but re-imported as {new}")]
    KindChanged {
        /// The offending key.
        key: String,
        /// Kind at first import.
        old: crate::AssetKind,
        /// Kind of the rejected import.
        new: crate::AssetKind,
    },
    /// Two different keys hashed to the same id (64-bit FNV collision —
    /// reported instead of silently corrupting the registry).
    #[error("asset id collision at {0}")]
    IdCollision(crate::AssetId),
    /// glTF import failure (with the glTF crate's own diagnostics).
    #[error("glTF import failed for {key:?}: {source}")]
    GlTF {
        /// Asset key being imported.
        key: String,
        /// Underlying gltf-rs error.
        source: gltf::Error,
    },
    /// The glTF document uses something the importer does not support.
    #[error("unsupported glTF feature: {0}")]
    UnsupportedGlTF(#[source] UnsupportedGlTF),
    /// A referenced accessor or buffer was missing from the document.
    #[error("glTF {key:?}: missing {what} (node/skin/mesh index {index})")]
    MissingGlTFData {
        /// Asset key being imported.
        key: String,
        /// What was missing (e.g. "POSITION accessor").
        what: String,
        /// Which object referenced it.
        index: usize,
    },
    /// Filesystem I/O at the registry boundary (import_file).
    #[error("asset I/O for {key:?}: {source}")]
    Io {
        /// Asset key being imported.
        key: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// The subset of glTF 2.0 this importer accepts, and why it stopped.
///
/// glTF is a large standard; the importer is explicit about what it
/// reads so unsupported content fails loudly instead of importing
/// wrong data. See ADR-0020 for the scope rationale.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UnsupportedGlTF {
    /// Only `LINEAR` and `STEP` animation samplers are imported;
    /// `CUBICSPLINE` needs Hermite evaluation that is not part of the
    /// deterministic core yet.
    #[error("CUBICSPLINE animation sampler (use LINEAR or STEP)")]
    CubicSplineSampler,
    /// Animation channels targeting non-node properties (e.g. morph
    /// weights via KHR_animation_pointer) are out of scope.
    #[error("animation channel targets node TRS only")]
    NonNodeChannel,
    /// Only meshes with triangle-list primitives are imported (glTF's
    /// only defined mode), enforced defensively.
    #[error("non triangle-list primitive mode {0:?}")]
    NonTriangleMode(gltf::mesh::Mode),
    /// Importing this texture format is not supported by the core.
    #[error("texture import is not part of the Phase 5 core (meshes, skins, clips are)")]
    Textures,
}

/// Convenience alias for asset results.
pub type AssetResult<T> = Result<T, AssetError>;
