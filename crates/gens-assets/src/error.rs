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
    #[error("glTF {key:?}: missing {what} (index {index})")]
    MissingGlTFData {
        /// Asset key being imported.
        key: String,
        /// What was missing (e.g. "POSITION accessor").
        what: String,
        /// Which object referenced it.
        index: usize,
    },
    /// Imported mesh geometry failed validation (indices out of range,
    /// layer length mismatch, ...).
    #[error("glTF {key:?}: mesh {index} invalid: {reason}")]
    MeshInvalid {
        /// Asset key being imported.
        key: String,
        /// Mesh index.
        index: usize,
        /// What the validator rejected.
        reason: String,
    },
    /// glTF meshes with more than one primitive are rejected (a mesh
    /// with two materials imports as two draws the scene graph cannot
    /// yet express — fail loudly instead).
    #[error("glTF mesh {0} has multiple primitives (split it into per-primitive meshes)")]
    MultiPrimitive(usize),
    /// Skeleton construction from the skin failed.
    #[error("glTF {key:?}: skeleton import failed: {source}")]
    Skeleton {
        /// Asset key being imported.
        key: String,
        /// Underlying gens-animation error.
        source: gens_animation::SkeletonError,
    },
    /// Clip or track construction failed (bad duration, unsorted keys).
    #[error("glTF {key:?}: clip import failed: {source}")]
    Clip {
        /// Asset key being imported.
        key: String,
        /// Underlying gens-animation error.
        source: gens_animation::ClipError,
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
    /// Data-document import failure (parse or validation, Phase 6).
    #[error("data import failed for {key:?}: {source}")]
    Data {
        /// Asset key being imported.
        key: String,
        /// What the parser or validator rejected.
        #[source]
        source: DataError,
    },
}

/// Why a data document was rejected.
///
/// Every rule is loud: a document that would import "surprisingly"
/// (wrong version, duplicate names, degenerate transforms) fails here
/// instead of producing a world nobody asked for. See module docs of
/// [`crate::data`] for the format contract.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DataError {
    /// The document's `format_version` is not [`crate::data::DATA_FORMAT_VERSION`].
    #[error(
        "unsupported data document format version {found} (supported: {})",
        crate::data::DATA_FORMAT_VERSION
    )]
    UnsupportedVersion {
        /// Version found in the document.
        found: u32,
    },
    /// The document-level name is empty.
    #[error("document name must not be empty")]
    EmptyDocumentName,
    /// An entity has an empty name.
    #[error("entity at index {index} has an empty name")]
    EmptyEntityName {
        /// Position in the authored entity list.
        index: usize,
    },
    /// Entity names are unique within a document (the runtime bridge
    /// and scripts cross-reference by name).
    #[error("duplicate entity name {name:?} (names are unique within a document)")]
    DuplicateEntityName {
        /// The repeated name.
        name: String,
    },
    /// A translation component is NaN or infinite.
    #[error("entity {name:?}: translation contains a non-finite component")]
    NonFiniteTranslation {
        /// Offending entity.
        name: String,
    },
    /// A rotation component is NaN or infinite.
    #[error("entity {name:?}: rotation contains a non-finite component")]
    NonFiniteRotation {
        /// Offending entity.
        name: String,
    },
    /// The quaternion's norm deviates from 1 beyond tolerance.
    #[error(
        "entity {name:?}: rotation norm {norm} deviates from 1 by more than {tol} (author unit quaternions)"
    )]
    NonUnitRotation {
        /// Offending entity.
        name: String,
        /// Measured norm.
        norm: f32,
        /// The tolerance that was exceeded ([`crate::data::ROTATION_NORM_TOL`]).
        tol: f32,
    },
    /// A scale component is NaN or infinite.
    #[error("entity {name:?}: scale contains a non-finite component")]
    NonFiniteScale {
        /// Offending entity.
        name: String,
    },
    /// A scale component is exactly zero (collapses geometry;
    /// produces non-invertible matrices downstream).
    #[error(
        "entity {name:?}: scale has a zero component (use an epsilon; zero collapses geometry)"
    )]
    ZeroScale {
        /// Offending entity.
        name: String,
    },
    /// The script binding has an empty registry key.
    #[error("entity {name:?}: script key is empty")]
    EmptyScriptKey {
        /// Offending entity.
        name: String,
    },
    /// The script binding has an empty entry name.
    #[error("entity {name:?}: script entry is empty")]
    EmptyScriptEntry {
        /// Offending entity.
        name: String,
    },
    /// The bytes are not valid JSON.
    #[error("document is not valid JSON: {source}")]
    Json {
        /// serde_json's own diagnostics.
        #[from]
        source: serde_json::Error,
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
    /// Morph targets are not part of the Phase 5 core.
    #[error("morph target meshes (use a plain triangle mesh)")]
    MorphTargets,
    /// Nodes must be authored with TRS; a raw matrix node cannot be
    /// decomposed back to deterministic TRS without drift.
    #[error("node authored with a matrix transform (use translation/rotation/scale)")]
    MatrixTransform,
    /// Animation channels targeting morph weights are out of scope.
    #[error("animation channel targets morph weights (node TRS only)")]
    MorphWeightsChannel,
    /// One animation must drive one skin (a clip is bound to a skeleton).
    #[error("animation channels span multiple skins (one skin per clip)")]
    MixedSkinChannels,
    /// Importing this texture format is not supported by the core.
    #[error("texture import is not part of the Phase 5 core (meshes, skins, clips are)")]
    Textures,
}

/// Convenience alias for asset results.
pub type AssetResult<T> = Result<T, AssetError>;
