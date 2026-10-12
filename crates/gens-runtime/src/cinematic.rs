//! Cinematic playback integration (Phase 9, ADR-0030).
//!
//! The bridge between the pure [`gens_cinematics`] data model and the
//! live ECS world:
//!
//! - [`CinematicStage`] — the embedding layer's playback state: one
//!   active player plus the logical-target → entity binding map.
//! - [`advance_and_apply`] — advances the player by the caller's fixed
//!   delta and writes the sampled channels into bound entities:
//!   `Translation`/`Rotation`/`Scale` land in the entity's
//!   [`Transform`], `Fov` in the bound camera's [`CameraPose`], and
//!   `Fade` scalars are returned in the outcome (the embedding layer
//!   maps them to its own effect). Fired events come back in the
//!   outcome too — the caller routes them.
//! - [`CameraPose`] — the explicit camera staging the director
//!   contract requires (ADR-0026's lesson): while a cinematic owns a
//!   camera target, its pose is authoritative; the renderer/editor
//!   consumes this component, nothing samples a camera behind the
//!   director's back.
//!
//! Determinism: this module adds no clocks and no hidden state. Every
//! write is a pure function of `(stage, world, dt)`, and the
//! `cinematic_hello` example proves seek-equivalence — a playback
//! resumed from a seek hashes identically to the straight-through run.

use std::collections::BTreeMap;

use gens_cinematics::{Channel, CinematicPlayer, PlaybackState, SampledValue, TimelineEvent};
use gens_ecs::ecs::component::Component;
use gens_ecs::ecs::entity::Entity;
use thiserror::Error;

use gens_ecs::Transform;

/// Errors from applying a cinematic to a live world.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CinematicApplyError {
    /// A track's target has no binding.
    #[error("track target '{target}' is not bound to an entity")]
    UnboundTarget {
        /// The unbound logical target name.
        target: String,
    },
    /// A bound entity lacks the component the channel writes.
    #[error("bound entity for '{target}' lacks a Transform component")]
    MissingTransform {
        /// The logical target name.
        target: String,
    },
    /// The player rejected the delta (non-finite, negative, overflow).
    #[error("player rejected the advance: {0}")]
    Player(#[from] gens_cinematics::PlayerError),
}

/// What one `advance_and_apply` did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApplyOutcome {
    /// Events crossed during this advance, in firing order.
    pub events: Vec<TimelineEvent>,
    /// `Fade` channel samples per logical target (embedding maps them).
    pub fades: Vec<(String, f32)>,
    /// True when the cinematic is currently playing (and thus staging
    /// the camera, if the timeline declares one).
    pub camera_live: bool,
}

/// Explicit camera staging component (ADR-0026 explicit-staging rule).
///
/// Written by [`advance_and_apply`] for the timeline's camera target;
/// a renderer or the editor reads it to build the view. Fields are the
/// camera's own pose plus the directed FOV in degrees.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Camera world-space position.
    pub position: gens_math::Vec3,
    /// Camera world-space rotation.
    pub rotation: gens_math::Quat,
    /// Vertical field of view in degrees.
    pub fov_degrees: f32,
}

/// The embedding layer's cinematic playback state.
///
/// One active cinematic at a time (the director contract: two players
/// staging the same camera would fight; a phase that wants overlap
/// gives each cinematic its own camera target and stage).
#[derive(Debug, Clone)]
pub struct CinematicStage {
    /// The active player (idle players apply nothing).
    pub player: CinematicPlayer,
    /// Logical target name → entity, ordered by name (deterministic).
    pub bindings: BTreeMap<String, Entity>,
}

impl CinematicStage {
    /// Builds a stage from a player and a binding iterator.
    ///
    /// # Errors
    /// [`CinematicApplyError::UnboundTarget`] is NOT checked here —
    /// bindings may legitimately cover a subset of targets (a timeline
    /// whose Fade target has no entity is an embedding-layer decision);
    /// every track that actually writes checks at apply time.
    #[must_use]
    pub fn new(
        player: CinematicPlayer,
        bindings: impl IntoIterator<Item = (String, Entity)>,
    ) -> CinematicStage {
        CinematicStage {
            player,
            bindings: bindings.into_iter().collect(),
        }
    }

    /// The entity bound to `target`, if any.
    #[must_use]
    pub fn bound(&self, target: &str) -> Option<Entity> {
        self.bindings.get(target).copied()
    }
}

/// Advances the stage's player by `dt` and writes sampled channels.
///
/// Channels map as documented on [`CinematicStage`]. The apply order is
/// deliberately **advance → sample → validate → write**: the frame's
/// world state must reflect the time the step ENDED at (a fixed-step
/// loop at step N writes the state of time N*dt, not (N-1)*dt), so the
/// player moves first and sampling happens at the new local time.
/// Writes are validated against the world before any component lands;
/// on a validation failure the player clock and state are rewound to
/// the pre-advance values (seek is exact and fires nothing), so a
/// rejected apply is a genuine no-op — a replay can never desync on a
/// failed frame. Multiple channels on one entity fold field-by-field in
/// deterministic `(target, channel)` sample order and insert once per
/// entity. Non-playing players are a documented no-op (a paused
/// cinematic holds the world at its last written frame).
///
/// # Errors
/// [`CinematicApplyError`] — unbound targets for transform/fov
/// channels, bound entities missing their components, or the player
/// rejecting `dt`.
pub fn advance_and_apply(
    stage: &mut CinematicStage,
    world: &mut gens_ecs::ecs::world::World,
    dt: f32,
) -> Result<ApplyOutcome, CinematicApplyError> {
    let mut outcome = ApplyOutcome::default();

    // Non-playing players are a no-op (documented): nothing is sampled,
    // nothing is written.
    if stage.player.state() != PlaybackState::Playing {
        return Ok(outcome);
    }

    // 1. ADVANCE first: the frame's writes must reflect the time the
    //    step ended at, so the clock moves before sampling. The clock
    //    and state are remembered so a validation failure below can
    //    rewind the apply into a true no-op.
    let prev_state = stage.player.state();
    let prev_time = stage.player.local_time();
    outcome.events = stage.player.advance(dt)?;
    let sampling_time = stage.player.local_time();
    let samples = stage.player.timeline().sample_all(sampling_time);

    // 2. VALIDATE + fold: per entity, one Transform folded field-by-field
    //    in sample order, and one CameraPose per camera entity.
    let mut transform_folds: BTreeMap<Entity, Transform> = BTreeMap::new();
    let mut pose_folds: BTreeMap<Entity, CameraPose> = BTreeMap::new();

    for (address, value) in samples {
        match (address.channel, value) {
            (Channel::Fade, SampledValue::Float(f)) => {
                outcome.fades.push((address.target.clone(), f));
            }
            (Channel::Fov, SampledValue::Float(f)) => {
                let entity = stage.bound(&address.target).ok_or_else(|| {
                    CinematicApplyError::UnboundTarget {
                        target: address.target.clone(),
                    }
                })?;
                // Seed the fold from the live CameraPose, else from the
                // live Transform (position/rotation), else it's an error.
                let pose = if let Some(existing) = world.get::<CameraPose>(entity) {
                    CameraPose {
                        fov_degrees: f,
                        ..*existing
                    }
                } else if let Some(t) = world.get::<Transform>(entity) {
                    CameraPose {
                        position: t.position,
                        rotation: t.rotation,
                        fov_degrees: f,
                    }
                } else {
                    return Err(CinematicApplyError::MissingTransform {
                        target: address.target.clone(),
                    });
                };
                pose_folds.insert(entity, pose);
            }
            (Channel::Translation | Channel::Scale, SampledValue::Vec3(v)) => {
                let entity = stage.bound(&address.target).ok_or_else(|| {
                    CinematicApplyError::UnboundTarget {
                        target: address.target.clone(),
                    }
                })?;
                let t = transform_folds.entry(entity).or_insert_with(|| {
                    // Reading here is safe: validation may only FAIL,
                    // never write.
                    *world
                        .get::<Transform>(entity)
                        .unwrap_or(&Transform::IDENTITY)
                });
                match address.channel {
                    Channel::Translation => t.position = v,
                    Channel::Scale => t.scale = v,
                    _ => unreachable!("only Translation/Scale reach this arm"),
                }
            }
            (Channel::Rotation, SampledValue::Quat(q)) => {
                let entity = stage.bound(&address.target).ok_or_else(|| {
                    CinematicApplyError::UnboundTarget {
                        target: address.target.clone(),
                    }
                })?;
                let t = transform_folds.entry(entity).or_insert_with(|| {
                    *world
                        .get::<Transform>(entity)
                        .unwrap_or(&Transform::IDENTITY)
                });
                t.rotation = q;
            }
            // Mismatched kinds are rejected by the timeline builder;
            // these arms document the unreachable shapes loudly.
            (Channel::Translation | Channel::Scale, SampledValue::Quat(_))
            | (Channel::Rotation, SampledValue::Vec3(_))
            | (Channel::Translation | Channel::Rotation | Channel::Scale, SampledValue::Float(_))
            | (Channel::Fov, SampledValue::Vec3(_) | SampledValue::Quat(_))
            | (Channel::Fade, SampledValue::Vec3(_) | SampledValue::Quat(_)) => {
                unreachable!("timeline builder kind validation prevents mismatched channels");
            }
        }
    }

    // 3. VALIDATE against the live world (loud, before any write).
    let validation: Option<CinematicApplyError> = {
        let mut failure = None;
        for &entity in transform_folds.keys() {
            if world.get::<Transform>(entity).is_none() {
                let target = stage
                    .bindings
                    .iter()
                    .find(|(_, bound)| **bound == entity)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                failure = Some(CinematicApplyError::MissingTransform { target });
                break;
            }
        }
        failure
    };
    if let Some(err) = validation {
        // Rewind the clock and state: the rejected apply is a no-op.
        stage
            .player
            .seek(prev_time)
            .expect("previous local time is valid");
        match prev_state {
            PlaybackState::Playing => stage.player.play(),
            PlaybackState::Paused => stage.player.pause(),
            other => {
                let _ = other; // Idle/Finished cannot reach the advance path
            }
        }
        return Err(err);
    }

    // 4. WRITE.
    for (entity, t) in transform_folds {
        world.entity_mut(entity).insert(t);
    }
    for (entity, pose) in pose_folds {
        world.entity_mut(entity).insert(pose);
    }

    outcome.camera_live = true;

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_cinematics::timeline::{CameraCut, Timeline};
    use gens_cinematics::track::{FloatKey, FloatTrack, Interpolation, Vec3Key, Vec3Track};
    use gens_ecs::{Active, Name};
    use gens_math::{Vec3, vec3};

    fn translation_timeline(duration: f32, keys: &[(f32, [f32; 3])]) -> gens_cinematics::Timeline {
        Timeline::builder(duration)
            .vec3_track(
                "hero",
                Channel::Translation,
                Vec3Track::new(
                    keys.iter()
                        .map(|&(t, v)| Vec3Key {
                            time: t,
                            value: vec3(v[0], v[1], v[2]),
                        })
                        .collect(),
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("track ok")
            .build()
            .expect("timeline ok")
    }

    #[test]
    fn writes_transform_channels_into_bound_entities() {
        let timeline = translation_timeline(2.0, &[(0.0, [0.0; 3]), (2.0, [2.0, 0.0, 0.0])]);
        let mut world = gens_ecs::ecs::world::World::new();
        let hero = world
            .spawn((Name::new("Hero"), Transform::IDENTITY, Active))
            .id();
        let player = CinematicPlayer::new(std::sync::Arc::new(timeline));
        let mut stage = CinematicStage::new(player, [("hero".to_string(), hero)]);

        stage.player.play();
        let outcome = advance_and_apply(&mut stage, &mut world, 1.0).expect("apply ok");
        assert!(outcome.events.is_empty());
        let t = world.get::<Transform>(hero).unwrap();
        assert_eq!(t.position, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn unbound_targets_are_loud() {
        let timeline = translation_timeline(2.0, &[(0.0, [0.0; 3])]);
        let mut world = gens_ecs::ecs::world::World::new();
        let hero = world
            .spawn((Name::new("Hero"), Transform::IDENTITY, Active))
            .id();
        // Stage with NO bindings at all.
        let player = CinematicPlayer::new(std::sync::Arc::new(timeline));
        let mut stage = CinematicStage::new(player, std::iter::empty());
        stage.player.play();
        assert_eq!(
            advance_and_apply(&mut stage, &mut world, 0.5),
            Err(CinematicApplyError::UnboundTarget {
                target: "hero".to_string()
            })
        );
        // The world is untouched after the failed apply.
        assert_eq!(world.get::<Transform>(hero).unwrap().position, Vec3::ZERO);
    }

    #[test]
    fn fov_writes_camera_pose() {
        let timeline = Timeline::builder(2.0)
            .float_track(
                "cam",
                Channel::Fov,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 60.0,
                        },
                        FloatKey {
                            time: 2.0,
                            value: 40.0,
                        },
                    ],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("ok")
            .camera(CameraCut {
                target: "cam".to_string(),
                hard_cut: true,
                blend_in: 0.0,
            })
            .build()
            .unwrap();
        let mut world = gens_ecs::ecs::world::World::new();
        let cam = world
            .spawn((Name::new("Camera"), Transform::IDENTITY, Active))
            .id();
        let player = CinematicPlayer::new(std::sync::Arc::new(timeline));
        let mut stage = CinematicStage::new(player, [("cam".to_string(), cam)]);
        stage.player.play();
        advance_and_apply(&mut stage, &mut world, 1.0).expect("apply ok");
        let pose = world.get::<CameraPose>(cam).unwrap();
        assert_eq!(pose.fov_degrees, 50.0);
        // The camera's position came from its Transform at first write.
        assert_eq!(pose.position, Vec3::ZERO);
    }

    #[test]
    fn idle_players_apply_nothing() {
        let timeline = translation_timeline(2.0, &[(0.0, [0.0; 3]), (2.0, [1.0; 3])]);
        let mut world = gens_ecs::ecs::world::World::new();
        let hero = world
            .spawn((Name::new("Hero"), Transform::IDENTITY, Active))
            .id();
        let player = CinematicPlayer::new(std::sync::Arc::new(timeline));
        let mut stage = CinematicStage::new(player, [("hero".to_string(), hero)]);
        // Never played: advance is a no-op on both player and world.
        let outcome = advance_and_apply(&mut stage, &mut world, 0.5).expect("ok");
        assert!(outcome.events.is_empty());
        assert!(!outcome.camera_live);
        assert_eq!(world.get::<Transform>(hero).unwrap().position, Vec3::ZERO);
    }

    #[test]
    fn fades_come_back_in_the_outcome() {
        let timeline = Timeline::builder(2.0)
            .float_track(
                "overlay",
                Channel::Fade,
                FloatTrack::new(
                    vec![
                        FloatKey {
                            time: 0.0,
                            value: 0.0,
                        },
                        FloatKey {
                            time: 2.0,
                            value: 1.0,
                        },
                    ],
                    Interpolation::Linear,
                )
                .unwrap(),
            )
            .expect("ok")
            .build()
            .unwrap();
        let mut world = gens_ecs::ecs::world::World::new();
        let player = CinematicPlayer::new(std::sync::Arc::new(timeline));
        // Fade has no entity — the embedding layer decided that.
        let mut stage = CinematicStage::new(player, std::iter::empty());
        stage.player.play();
        let outcome = advance_and_apply(&mut stage, &mut world, 1.0).expect("ok");
        assert_eq!(outcome.fades, vec![("overlay".to_string(), 0.5)]);
    }
}
