//! glTF 2.0 import into engine types (Phase 5).
//!
//! Reads a glTF document and produces engine-owned objects:
//!
//! - node trees ([`ImportedNode`]) with deterministic TRS;
//! - meshes ([`ImportedMesh`]) — triangle lists with optional skin
//!   attributes (JOINTS_0/WEIGHTS_0 parsed now so the skinned GPU path
//!   can adopt them without re-importing, ADR-0016);
//! - skins ([`ImportedSkin`]) — gens-animation [`Skeleton`]s plus the
//!   inverse-bind matrices (the Skeleton type stores none; they are a
//!   skinned-render concern);
//! - animation clips ([`ImportedClip`]) — gens-animation [`Clip`]s
//!   bound to the skin whose joints they animate.
//!
//! The accepted subset is explicit (see [`UnsupportedGlTF`]): LINEAR
//! and STEP samplers only, single-primitive triangle meshes only, TRS
//! nodes only, node-TRS animation only, one skin per clip. Everything
//! unsupported fails loudly — a silent partial import would be a fake
//! implementation.
//!
//! Determinism: importing the same bytes always yields the same
//! objects (pure conversion below the file boundary); names and joint
//! orders come from the document, not the environment.

use crate::error::{AssetError, AssetResult, UnsupportedGlTF};
use crate::mesh::MeshData;
use gens_animation::{
    Clip, Interpolation, JointTrack, QuatKey, QuatTrack, Skeleton, VecKey, VecTrack,
};
use gens_math::{Mat4, Quat, Vec3};

/// One imported glTF node: name, local TRS and links.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedNode {
    /// Node name (`node_<i>` when the document omits one).
    pub name: String,
    /// Local translation.
    pub translation: Vec3,
    /// Local rotation (glTF's x,y,z,w quaternion).
    pub rotation: Quat,
    /// Local scale.
    pub scale: Vec3,
    /// Index into [`ImportedGlTF::meshes`] when the node carries a mesh.
    pub mesh: Option<usize>,
    /// Index into [`ImportedGlTF::skins`] when the node is skinned.
    pub skin: Option<usize>,
    /// Child node indices in document order.
    pub children: Vec<usize>,
}

/// One imported mesh (a glTF mesh with exactly one primitive).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedMesh {
    /// Mesh name (`mesh_<i>` when omitted).
    pub name: String,
    /// The geometry.
    pub mesh: MeshData,
}

/// One imported skin: the skeleton plus its inverse-bind matrices.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedSkin {
    /// Skin name (`skin_<i>` when omitted).
    pub name: String,
    /// Joint hierarchy (joint order == glTF joint order == clip space).
    pub skeleton: Skeleton,
    /// One inverse-bind matrix per joint (identity when the document
    /// omits them, per glTF spec).
    pub inverse_bind_matrices: Vec<Mat4>,
    /// The glTF node index of each joint (rank == skeleton joint index).
    /// Kept so callers can map scene-graph nodes to joints.
    pub joint_nodes: Vec<usize>,
}

/// One imported animation bound to the skin it drives.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedClip {
    /// Clip name (`anim_<i>` when omitted).
    pub name: String,
    /// Index into [`ImportedGlTF::skins`] whose joint indices the
    /// clip's tracks address.
    pub skin: usize,
    /// The clip (duration = the largest key time).
    pub clip: Clip,
}

/// Everything one glTF document imported into.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedGlTF {
    /// All nodes, document order (index == glTF node index).
    pub nodes: Vec<ImportedNode>,
    /// All meshes, document order.
    pub meshes: Vec<ImportedMesh>,
    /// All skins, document order.
    pub skins: Vec<ImportedSkin>,
    /// All animations, document order.
    pub clips: Vec<ImportedClip>,
}

/// Imports a self-contained glTF from bytes (JSON with data-URI buffers
/// or a binary .glb). No filesystem access — CI fixtures use this path.
pub fn import_gltf_bytes(key: &str, bytes: &[u8]) -> AssetResult<ImportedGlTF> {
    let (doc, buffers, _images) = gltf::import_slice(bytes).map_err(|source| AssetError::GlTF {
        key: key.to_string(),
        source,
    })?;
    convert(key, &doc, &buffers)
}

/// Imports a glTF file with full buffer resolution (external .bin
/// files, relative URIs) — the production loading path.
pub fn import_gltf_file(key: &str, path: impl AsRef<std::path::Path>) -> AssetResult<ImportedGlTF> {
    let (doc, buffers, _images) = gltf::import(path).map_err(|source| AssetError::GlTF {
        key: key.to_string(),
        source,
    })?;
    convert(key, &doc, &buffers)
}

/// Converts a parsed document + resolved buffers into engine objects.
fn convert(
    key: &str,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> AssetResult<ImportedGlTF> {
    let nodes = import_nodes(doc)?;
    let meshes = import_meshes(key, doc, buffers)?;
    let skins = import_skins(key, doc, buffers)?;
    let clips = import_animations(key, doc, buffers, &skins)?;
    Ok(ImportedGlTF {
        nodes,
        meshes,
        skins,
        clips,
    })
}

/// Buffer resolver: document buffer index -> its bytes.
fn resolver<'a>(
    buffers: &'a [gltf::buffer::Data],
) -> impl Fn(gltf::Buffer) -> Option<&'a [u8]> + Clone + 'a {
    move |buf: gltf::Buffer| buffers.get(buf.index()).map(|d| &d[..])
}

/// Reads all nodes as TRS (matrix-authored nodes are rejected — see
/// [`UnsupportedGlTF::MatrixTransform`]).
fn import_nodes(doc: &gltf::Document) -> AssetResult<Vec<ImportedNode>> {
    let mut nodes = Vec::with_capacity(doc.nodes().count());
    for (i, node) in doc.nodes().enumerate() {
        let (translation, rotation, scale) = match node.transform() {
            gltf::scene::Transform::Decomposed {
                translation,
                rotation,
                scale,
            } => (
                Vec3::from(translation),
                Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
                Vec3::from(scale),
            ),
            gltf::scene::Transform::Matrix { .. } => {
                return Err(AssetError::UnsupportedGlTF(
                    UnsupportedGlTF::MatrixTransform,
                ));
            }
        };
        nodes.push(ImportedNode {
            name: node
                .name()
                .map_or_else(|| format!("node_{i}"), str::to_owned),
            translation,
            rotation,
            scale,
            mesh: node.mesh().map(|m| m.index()),
            skin: node.skin().map(|s| s.index()),
            children: node.children().map(|c| c.index()).collect(),
        });
    }
    Ok(nodes)
}

/// Reads every mesh (exactly one primitive per mesh is accepted; a
/// multi-primitive mesh imports as multiple draws the scene graph
/// cannot yet express, so it is rejected loudly).
fn import_meshes(
    key: &str,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> AssetResult<Vec<ImportedMesh>> {
    let mut meshes = Vec::with_capacity(doc.meshes().count());
    for (i, m) in doc.meshes().enumerate() {
        let prims: Vec<_> = m.primitives().collect();
        if prims.len() != 1 {
            return Err(AssetError::MultiPrimitive(i));
        }
        let mesh = read_primitive(key, i, &prims[0], resolver(buffers))?;
        meshes.push(ImportedMesh {
            name: m.name().map_or_else(|| format!("mesh_{i}"), str::to_owned),
            mesh,
        });
    }
    Ok(meshes)
}

/// Reads one primitive's attributes into a [`MeshData`].
fn read_primitive<'a>(
    key: &str,
    mesh_index: usize,
    prim: &gltf::mesh::Primitive<'_>,
    get_data: impl Fn(gltf::Buffer) -> Option<&'a [u8]> + Clone,
) -> AssetResult<MeshData> {
    if prim.mode() != gltf::mesh::Mode::Triangles {
        return Err(AssetError::UnsupportedGlTF(
            UnsupportedGlTF::NonTriangleMode(prim.mode()),
        ));
    }
    if prim.morph_targets().next().is_some() {
        return Err(AssetError::UnsupportedGlTF(UnsupportedGlTF::MorphTargets));
    }
    let reader = prim.reader(get_data);
    let positions: Vec<f32> = reader
        .read_positions()
        .ok_or_else(|| AssetError::MissingGlTFData {
            key: key.to_string(),
            what: "POSITION accessor".to_string(),
            index: mesh_index,
        })?
        .flatten()
        .collect();
    // gltf 1.4 hands normals over as f32 triples (internally widened).
    let normals: Vec<f32> = reader
        .read_normals()
        .map(|it| it.flatten().collect())
        .unwrap_or_default();
    let uvs: Vec<f32> = match reader.read_tex_coords(0) {
        Some(gltf::mesh::util::ReadTexCoords::U8(it)) => it
            .flat_map(|v| [f32::from(v[0]) / 255.0, f32::from(v[1]) / 255.0])
            .collect(),
        Some(gltf::mesh::util::ReadTexCoords::U16(it)) => it
            .flat_map(|v| [f32::from(v[0]) / 65535.0, f32::from(v[1]) / 65535.0])
            .collect(),
        Some(gltf::mesh::util::ReadTexCoords::F32(it)) => it.flatten().collect(),
        None => Vec::new(),
    };
    let joints: Vec<u16> = match reader.read_joints(0) {
        Some(gltf::mesh::util::ReadJoints::U8(it)) => it.flat_map(|v| v.map(u16::from)).collect(),
        Some(gltf::mesh::util::ReadJoints::U16(it)) => it.flatten().collect(),
        None => Vec::new(),
    };
    let weights: Vec<f32> = match reader.read_weights(0) {
        Some(gltf::mesh::util::ReadWeights::U8(it)) => {
            it.flat_map(|v| v.map(|c| f32::from(c) / 255.0)).collect()
        }
        Some(gltf::mesh::util::ReadWeights::U16(it)) => {
            it.flat_map(|v| v.map(|c| f32::from(c) / 65535.0)).collect()
        }
        Some(gltf::mesh::util::ReadWeights::F32(it)) => it.flatten().collect(),
        None => Vec::new(),
    };
    let indices: Vec<u32> = match reader.read_indices() {
        Some(gltf::mesh::util::ReadIndices::U8(it)) => it.map(u32::from).collect(),
        Some(gltf::mesh::util::ReadIndices::U16(it)) => it.map(u32::from).collect(),
        Some(gltf::mesh::util::ReadIndices::U32(it)) => it.collect(),
        None => Vec::new(),
    };
    let mesh = MeshData {
        positions,
        normals,
        uvs,
        joints,
        weights,
        indices,
    };
    if let Err(e) = mesh.validate() {
        return Err(AssetError::MeshInvalid {
            key: key.to_string(),
            index: mesh_index,
            reason: e.to_string(),
        });
    }
    Ok(mesh)
}

/// Reads all skins as skeletons + inverse-bind matrices.
fn import_skins(
    key: &str,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> AssetResult<Vec<ImportedSkin>> {
    // Parent map from scene traversal (a joint whose parent is outside
    // every scene, or not itself a joint, is a root joint). Iteration
    // order is deterministic (scenes/nodes in document order) and the
    // map is keyed by node index, so the result is order-independent.
    let mut parent_of: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for scene in doc.scenes() {
        for root in scene.nodes() {
            let mut stack = vec![root.index()];
            while let Some(idx) = stack.pop() {
                let children: Vec<usize> = doc
                    .nodes()
                    .nth(idx)
                    .expect("scene node index valid")
                    .children()
                    .map(|c| c.index())
                    .collect();
                for &c in &children {
                    parent_of.insert(c, idx);
                    stack.push(c);
                }
            }
        }
    }

    let mut skins = Vec::with_capacity(doc.skins().count());
    for (i, skin) in doc.skins().enumerate() {
        let joint_nodes: Vec<usize> = skin.joints().map(|n| n.index()).collect();
        let joint_rank: std::collections::BTreeMap<usize, usize> = joint_nodes
            .iter()
            .enumerate()
            .map(|(rank, &node)| (node, rank))
            .collect();
        let names: Vec<String> = joint_nodes
            .iter()
            .map(|&n| {
                doc.nodes()
                    .nth(n)
                    .expect("joint node index valid")
                    .name()
                    .map_or_else(|| format!("joint_{n}"), str::to_owned)
            })
            .collect();
        let parents: Vec<Option<usize>> = joint_nodes
            .iter()
            .map(|&n| parent_of.get(&n).and_then(|p| joint_rank.get(p).copied()))
            .collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let skeleton =
            Skeleton::new(&name_refs, &parents).map_err(|source| AssetError::Skeleton {
                key: key.to_string(),
                source,
            })?;
        let ibms: Vec<Mat4> = match skin.inverse_bind_matrices() {
            Some(acc) => {
                let mats: Vec<[[f32; 4]; 4]> =
                    gltf::accessor::util::Iter::new(acc, resolver(buffers))
                        .ok_or_else(|| AssetError::MissingGlTFData {
                            key: key.to_string(),
                            what: "inverse bind matrices (not f32 mat4)".to_string(),
                            index: i,
                        })?
                        .collect();
                if mats.len() != joint_nodes.len() {
                    return Err(AssetError::MissingGlTFData {
                        key: key.to_string(),
                        what: "inverse bind matrices (count mismatch)".to_string(),
                        index: i,
                    });
                }
                mats.into_iter()
                    .map(|m| Mat4::from_cols_array_2d(&m))
                    .collect()
            }
            None => (0..joint_nodes.len()).map(|_| Mat4::IDENTITY).collect(),
        };
        skins.push(ImportedSkin {
            name: skin
                .name()
                .map_or_else(|| format!("skin_{i}"), str::to_owned),
            skeleton,
            inverse_bind_matrices: ibms,
            joint_nodes,
        });
    }
    Ok(skins)
}

/// Reads all animations as clips bound to the skin they animate.
fn import_animations(
    key: &str,
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    skins: &[ImportedSkin],
) -> AssetResult<Vec<ImportedClip>> {
    let mut clips = Vec::with_capacity(doc.animations().count());
    for (i, anim) in doc.animations().enumerate() {
        let mut tracks: std::collections::BTreeMap<usize, JointTrack> =
            std::collections::BTreeMap::new();
        let mut duration = 0.0_f32;
        let mut clip_skin: Option<usize> = None;

        for channel in anim.channels() {
            let target = channel.target();
            let node = target.node().index();
            let property = target.property();
            let sampler = channel.sampler();
            let interpolation = match sampler.interpolation() {
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::CubicSpline => {
                    return Err(AssetError::UnsupportedGlTF(
                        UnsupportedGlTF::CubicSplineSampler,
                    ));
                }
            };
            let times: Vec<f32> =
                gltf::accessor::util::Iter::new(sampler.input(), resolver(buffers))
                    .ok_or_else(|| AssetError::MissingGlTFData {
                        key: key.to_string(),
                        what: "animation sampler input (not f32 scalars)".to_string(),
                        index: i,
                    })?
                    .collect();

            // Resolve the target node to a (skin, joint) pair, then
            // read the outputs with the item type the property needs.
            let (skin_idx, joint_idx) = resolve_joint(node, doc)
                .ok_or_else(|| AssetError::UnsupportedGlTF(UnsupportedGlTF::NonNodeChannel))?;
            match clip_skin {
                None => clip_skin = Some(skin_idx),
                Some(s) if s != skin_idx => {
                    return Err(AssetError::UnsupportedGlTF(
                        UnsupportedGlTF::MixedSkinChannels,
                    ));
                }
                _ => {}
            }

            let track = tracks.entry(joint_idx).or_default();
            match property {
                gltf::animation::Property::Translation => {
                    let outs: Vec<[f32; 3]> =
                        gltf::accessor::util::Iter::new(sampler.output(), resolver(buffers))
                            .ok_or_else(|| AssetError::MissingGlTFData {
                                key: key.to_string(),
                                what: "translation output (not f32 vec3)".to_string(),
                                index: i,
                            })?
                            .collect();
                    if outs.len() != times.len() {
                        return Err(AssetError::MissingGlTFData {
                            key: key.to_string(),
                            what: "input/output count mismatch".to_string(),
                            index: i,
                        });
                    }
                    let keys: Vec<VecKey> = times
                        .iter()
                        .zip(&outs)
                        .map(|(&t, o)| VecKey {
                            time: t,
                            value: Vec3::from(*o),
                        })
                        .collect();
                    if let Some(&t) = times.last() {
                        duration = duration.max(t);
                    }
                    track.translation =
                        Some(VecTrack::new(keys, interpolation).map_err(|source| {
                            AssetError::Clip {
                                key: key.to_string(),
                                source,
                            }
                        })?);
                }
                gltf::animation::Property::Rotation => {
                    let outs: Vec<[f32; 4]> =
                        gltf::accessor::util::Iter::new(sampler.output(), resolver(buffers))
                            .ok_or_else(|| AssetError::MissingGlTFData {
                                key: key.to_string(),
                                what: "rotation output (not f32 vec4)".to_string(),
                                index: i,
                            })?
                            .collect();
                    if outs.len() != times.len() {
                        return Err(AssetError::MissingGlTFData {
                            key: key.to_string(),
                            what: "input/output count mismatch".to_string(),
                            index: i,
                        });
                    }
                    let keys: Vec<QuatKey> = times
                        .iter()
                        .zip(&outs)
                        .map(|(&t, o)| QuatKey {
                            time: t,
                            value: Quat::from_xyzw(o[0], o[1], o[2], o[3]),
                        })
                        .collect();
                    if let Some(&t) = times.last() {
                        duration = duration.max(t);
                    }
                    track.rotation =
                        Some(QuatTrack::new(keys, interpolation).map_err(|source| {
                            AssetError::Clip {
                                key: key.to_string(),
                                source,
                            }
                        })?);
                }
                gltf::animation::Property::Scale => {
                    let outs: Vec<[f32; 3]> =
                        gltf::accessor::util::Iter::new(sampler.output(), resolver(buffers))
                            .ok_or_else(|| AssetError::MissingGlTFData {
                                key: key.to_string(),
                                what: "scale output (not f32 vec3)".to_string(),
                                index: i,
                            })?
                            .collect();
                    if outs.len() != times.len() {
                        return Err(AssetError::MissingGlTFData {
                            key: key.to_string(),
                            what: "input/output count mismatch".to_string(),
                            index: i,
                        });
                    }
                    let keys: Vec<VecKey> = times
                        .iter()
                        .zip(&outs)
                        .map(|(&t, o)| VecKey {
                            time: t,
                            value: Vec3::from(*o),
                        })
                        .collect();
                    if let Some(&t) = times.last() {
                        duration = duration.max(t);
                    }
                    track.scale = Some(VecTrack::new(keys, interpolation).map_err(|source| {
                        AssetError::Clip {
                            key: key.to_string(),
                            source,
                        }
                    })?);
                }
                gltf::animation::Property::MorphTargetWeights => {
                    return Err(AssetError::UnsupportedGlTF(
                        UnsupportedGlTF::MorphWeightsChannel,
                    ));
                }
            }
        }

        let name = anim
            .name()
            .map_or_else(|| format!("anim_{i}"), str::to_owned);
        let clip =
            Clip::new(name.clone(), duration, tracks).map_err(|source| AssetError::Clip {
                key: key.to_string(),
                source,
            })?;
        debug_assert!(clip_skin.is_some() || clips.is_empty());
        clips.push(ImportedClip {
            name,
            skin: clip_skin.unwrap_or(0),
            clip,
        });
    }
    let _ = skins;
    Ok(clips)
}

/// Maps an animated node index to (skin, joint) using the document's
/// skins (joint order == skeleton joint index by construction).
fn resolve_joint(node: usize, doc: &gltf::Document) -> Option<(usize, usize)> {
    for (s, skin) in doc.skins().enumerate() {
        for (j, joint) in skin.joints().enumerate() {
            if joint.index() == node {
                return Some((s, j));
            }
        }
    }
    None
}
