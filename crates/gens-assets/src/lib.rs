//! Asset import, processing, versioning and streaming (Phase 5).
//!
//! `gens-assets` is the engine's content layer. Phase 5 delivers the
//! **registry core** (this module) and the **glTF import** ([`gltf`]
//! module):
//!
//! - [`hash::fnv1a64`] — the content hash every asset is fingerprinted
//!   with (the same FNV-1a the engine's evidence ledger uses);
//! - [`registry::AssetRegistry`] — a deterministic, ordered map of
//!   assets keyed by [`registry::AssetId`], with path lookup, content
//!   hashing and version bumps on change;
//! - [`gltf::import_gltf`] — glTF 2.0 import into engine types: meshes
//!   ([`mesh::MeshData`]), skeletons + clips (gens-animation), and node
//!   transforms.
//!
//! Determinism contract: importing the same bytes always produces the
//! same ids, hashes and engine objects — asset identity is a pure
//! function of content and canonical key, never of time or environment.
//! File I/O happens only at the registry/import boundary; everything
//! below it is pure and unit-testable.

pub mod error;
pub mod hash;
pub mod mesh;
pub mod registry;

pub use error::AssetError;
pub use hash::fnv1a64;
pub use mesh::MeshData;
pub use registry::{AssetEntry, AssetId, AssetKind, AssetRegistry};
