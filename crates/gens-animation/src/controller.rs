//! Clip playback behind a small state machine.
//!
//! [`AnimationController`] owns named states (a clip + looping mode +
//! outgoing transitions), a declared-parameter table, and the crossfade
//! bookkeeping between states. Conditions are **data** (parameter
//! comparisons), not closures: the same parameter/update sequence always
//! produces the same pose sequence, which is what makes controllers
//! replay-testable (R-013).
//!
//! Semantics pinned here:
//!
//! - the **first added state is the initial state**;
//! - transitions are evaluated only while no crossfade is running, in
//!   insertion order, first match wins;
//! - a transition of `duration == 0` switches instantly; otherwise the
//!   outgoing clip keeps playing during the fade and the incoming clip
//!   starts at time 0;
//! - `dt <= 0` or non-finite `dt` is ignored (advance nothing) so a bad
//!   frame can never corrupt playback state.

use std::collections::BTreeMap;
use std::sync::Arc;

use gens_math::Vec3;
use thiserror::Error;

use crate::clip::Clip;
use crate::skeleton::LocalPose;

/// Identifier of a controller state (the state's name).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateId(String);

impl StateId {
    /// The state's name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A declared parameter value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    /// A numeric parameter (speed, distance, health...).
    Number(f32),
    /// A boolean parameter (grounded, attacking...).
    Flag(bool),
}

/// Numeric comparison operators for [`Condition::Number`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberOp {
    /// `param < value`
    Less,
    /// `param <= value`
    LessEq,
    /// `param > value`
    Greater,
    /// `param >= value`
    GreaterEq,
}

/// A transition gate: a data-described test against declared parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// Compare a numeric parameter.
    Number {
        /// Parameter name (must be declared).
        name: String,
        /// Comparison operator.
        op: NumberOp,
        /// Threshold to compare against.
        value: f32,
    },
    /// Test a boolean parameter.
    Flag {
        /// Parameter name (must be declared).
        name: String,
        /// Required value.
        is: bool,
    },
}

impl Condition {
    fn eval(&self, params: &BTreeMap<String, ParamValue>) -> bool {
        match self {
            Condition::Number { name, op, value } => {
                match params.get(name) {
                    Some(ParamValue::Number(actual)) => match op {
                        NumberOp::Less => actual < value,
                        NumberOp::LessEq => actual <= value,
                        NumberOp::Greater => actual > value,
                        NumberOp::GreaterEq => actual >= value,
                    },
                    // Undeclared or type-mismatched parameters fail closed.
                    _ => false,
                }
            }
            Condition::Flag { name, is } => params.get(name) == Some(&ParamValue::Flag(*is)),
        }
    }
}

/// A directed edge between states.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// Destination state.
    pub to: StateId,
    /// Crossfade length in seconds (`0` = instant switch).
    pub duration: f32,
    /// All conditions must hold for the transition to fire. An empty
    /// list means the transition fires unconditionally (first scan).
    pub conditions: Vec<Condition>,
}

impl Transition {
    /// Instant (`duration = 0`) transition gated by `conditions`.
    #[must_use]
    pub fn instant(to: StateId, conditions: Vec<Condition>) -> Self {
        Self {
            to,
            duration: 0.0,
            conditions,
        }
    }

    /// Crossfaded transition gated by `conditions`.
    #[must_use]
    pub fn crossfade_to(to: StateId, duration: f32, conditions: Vec<Condition>) -> Self {
        Self {
            to,
            duration,
            conditions,
        }
    }
}

/// Errors from controller construction and calls.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ControllerError {
    /// The referenced state does not exist.
    #[error("unknown state: {state}")]
    UnknownState {
        /// The offending state name.
        state: String,
    },
    /// A state with this name already exists.
    #[error("duplicate state: {state}")]
    DuplicateState {
        /// The offending state name.
        state: String,
    },
    /// A parameter referenced by a condition was never declared.
    #[error("undeclared parameter: {parameter}")]
    UnknownParameter {
        /// The offending parameter name.
        parameter: String,
    },
    /// The parameter was already declared.
    #[error("duplicate parameter: {parameter}")]
    DuplicateParameter {
        /// The offending parameter name.
        parameter: String,
    },
    /// Transitions from a state to itself are rejected.
    #[error("self-transition on state {state}")]
    SelfTransition {
        /// The offending state name.
        state: String,
    },
    /// Transition durations must be finite and non-negative.
    #[error("transition duration must be finite and >= 0, got {duration}")]
    InvalidDuration {
        /// The offending duration.
        duration: f32,
    },
}

#[derive(Debug)]
struct State {
    clip: Arc<Clip>,
    looping: bool,
    transitions: Vec<Transition>,
}

#[derive(Debug)]
struct Fade {
    target: StateId,
    elapsed: f32,
    duration: f32,
    incoming_time: f32,
}

/// Clip playback state machine over a fixed-size [`LocalPose`].
#[derive(Debug)]
pub struct AnimationController {
    states: BTreeMap<StateId, State>,
    current: StateId,
    play_time: f32,
    fade: Option<Fade>,
    params: BTreeMap<String, ParamValue>,
    joint_count: usize,
    scratch: LocalPose,
}

impl AnimationController {
    /// An empty controller over `joint_count` joints.
    #[must_use]
    pub fn new(joint_count: usize) -> Self {
        Self {
            states: BTreeMap::new(),
            current: StateId(String::new()),
            play_time: 0.0,
            fade: None,
            params: BTreeMap::new(),
            joint_count,
            scratch: LocalPose::identity(joint_count),
        }
    }

    /// Adds a state; the **first** state added becomes the initial state.
    ///
    /// # Errors
    /// [`ControllerError::DuplicateState`] when the name is taken.
    pub fn add_state(
        &mut self,
        name: impl Into<String>,
        clip: Clip,
        looping: bool,
    ) -> Result<StateId, ControllerError> {
        let id = StateId(name.into());
        if self.states.contains_key(&id) {
            return Err(ControllerError::DuplicateState {
                state: id.as_str().to_owned(),
            });
        }
        if self.states.is_empty() {
            self.current = id.clone();
            self.play_time = 0.0;
        }
        self.states.insert(
            id.clone(),
            State {
                clip: Arc::new(clip),
                looping,
                transitions: Vec::new(),
            },
        );
        Ok(id)
    }

    /// Declares a parameter with an initial value.
    ///
    /// # Errors
    /// [`ControllerError::DuplicateParameter`] when already declared.
    pub fn add_parameter(
        &mut self,
        name: impl Into<String>,
        initial: ParamValue,
    ) -> Result<(), ControllerError> {
        let name = name.into();
        if self.params.contains_key(&name) {
            return Err(ControllerError::DuplicateParameter { parameter: name });
        }
        self.params.insert(name, initial);
        Ok(())
    }

    /// Writes a parameter value used by condition checks on the next update.
    ///
    /// # Errors
    /// [`ControllerError::UnknownParameter`] when never declared.
    pub fn set_parameter(
        &mut self,
        name: impl Into<String>,
        value: ParamValue,
    ) -> Result<(), ControllerError> {
        let name = name.into();
        if !self.params.contains_key(&name) {
            return Err(ControllerError::UnknownParameter { parameter: name });
        }
        self.params.insert(name, value);
        Ok(())
    }

    /// Adds an outgoing transition from `from`.
    ///
    /// # Errors
    /// [`ControllerError::UnknownState`] when `from` or `transition.to`
    /// does not exist, [`ControllerError::SelfTransition`],
    /// [`ControllerError::InvalidDuration`], or
    /// [`ControllerError::UnknownParameter`] when a condition references
    /// an undeclared parameter.
    pub fn add_transition(
        &mut self,
        from: &StateId,
        transition: Transition,
    ) -> Result<(), ControllerError> {
        let Transition {
            to,
            duration,
            conditions,
        } = &transition;
        if !self.states.contains_key(from) {
            return Err(ControllerError::UnknownState {
                state: from.as_str().to_owned(),
            });
        }
        if !self.states.contains_key(to) {
            return Err(ControllerError::UnknownState {
                state: to.as_str().to_owned(),
            });
        }
        if from == to {
            return Err(ControllerError::SelfTransition {
                state: from.as_str().to_owned(),
            });
        }
        if !duration.is_finite() || *duration < 0.0 {
            return Err(ControllerError::InvalidDuration {
                duration: *duration,
            });
        }
        for condition in conditions {
            let parameter = match condition {
                Condition::Number { name, .. } | Condition::Flag { name, .. } => name,
            };
            if !self.params.contains_key(parameter) {
                return Err(ControllerError::UnknownParameter {
                    parameter: parameter.clone(),
                });
            }
        }
        self.states
            .get_mut(from)
            .expect("state existence checked above")
            .transitions
            .push(transition);
        Ok(())
    }

    /// The currently active state.
    #[must_use]
    pub fn current_state(&self) -> &StateId {
        &self.current
    }

    /// Playback time within the current clip (wraps for looping states).
    #[must_use]
    pub fn play_time(&self) -> f32 {
        self.play_time
    }

    /// `true` while a crossfade is in progress.
    #[must_use]
    pub fn is_transitioning(&self) -> bool {
        self.fade.is_some()
    }

    /// Advances the controller by `dt` seconds and writes the blended pose.
    ///
    /// The pose is **overwritten** from identity each call: the controller
    /// owns the full pose of whatever it animates. `dt <= 0` or non-finite
    /// `dt` leaves everything unchanged.
    pub fn update(&mut self, dt: f32, pose: &mut LocalPose) {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        debug_assert_eq!(pose.joint_count(), self.joint_count);

        // 1. Transition scan (only while idle), insertion order, first match.
        if self.fade.is_none() {
            let params = &self.params;
            let fire = self
                .states
                .get(&self.current)
                .expect("current state always exists")
                .transitions
                .iter()
                .find(|t| t.conditions.iter().all(|c| c.eval(params)))
                .cloned();
            if let Some(transition) = fire {
                if transition.duration == 0.0 {
                    self.current = transition.to;
                    self.play_time = 0.0;
                } else {
                    self.fade = Some(Fade {
                        target: transition.to,
                        elapsed: 0.0,
                        duration: transition.duration,
                        incoming_time: 0.0,
                    });
                }
            }
        }

        // 2. Advance clocks.
        let mut finished: Option<StateId> = None;
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            fade.incoming_time += dt;
            self.play_time += dt;
            if fade.elapsed >= fade.duration {
                finished = Some(fade.target.clone());
            }
        } else {
            self.play_time += dt;
        }

        // 3. Sample + blend.
        if let Some(fade) = &mut self.fade {
            let outgoing = self
                .states
                .get(&self.current)
                .expect("current state always exists");
            self.scratch.set_identity();
            sample_clip(
                &outgoing.clip,
                outgoing.looping,
                self.play_time,
                &mut self.scratch,
            );

            let weight = if fade.duration > 0.0 {
                (fade.elapsed / fade.duration).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let incoming = self
                .states
                .get(&fade.target)
                .expect("transition target validated at add time");
            pose.set_identity();
            sample_clip(&incoming.clip, incoming.looping, fade.incoming_time, pose);
            // pose = lerp(incoming, outgoing, 1 - weight)
            crate::blend::blend_into(pose, &self.scratch, 1.0 - weight);

            if let Some(target) = finished {
                let incoming_time = fade.incoming_time;
                self.current = target;
                self.play_time = incoming_time;
                self.fade = None;
                // Keep play_time in [0, duration) for looping targets.
                if let Some(state) = self.states.get(&self.current) {
                    let d = state.clip.duration();
                    if state.looping && d > 0.0 {
                        self.play_time %= d;
                    }
                }
            }
        } else {
            // Clock was already advanced in step 2; wrap for looping states
            // so external queries see the same phase the sampler uses.
            let state = self
                .states
                .get(&self.current)
                .expect("current state always exists");
            if state.looping {
                let d = state.clip.duration();
                if d > 0.0 {
                    self.play_time %= d;
                }
            }
            pose.set_identity();
            sample_clip(&state.clip, state.looping, self.play_time, pose);
        }
    }
}

fn sample_clip(clip: &Clip, looping: bool, time: f32, pose: &mut LocalPose) {
    if looping {
        clip.sample_looped(time, pose);
    } else {
        clip.sample_clamped(time, pose);
    }
}

/// Convenience for building a translation-only clip on joint 0 — the
/// shape every controller test in this crate plays with.
#[must_use]
pub fn slide_clip(name: &str, duration: f32, from: Vec3, to: Vec3) -> Clip {
    Clip::translation(
        name,
        duration,
        0,
        vec![
            crate::clip::VecKey {
                time: 0.0,
                value: from,
            },
            crate::clip::VecKey {
                time: duration,
                value: to,
            },
        ],
    )
    .expect("two-key clip with positive duration is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::vec3;

    const EPS: f32 = 1e-4;

    fn controller_pair() -> AnimationController {
        let walk = slide_clip("walk", 1.0, vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0));
        let idle = slide_clip("idle", 1.0, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0));
        let mut c = AnimationController::new(1);
        // First added = initial state.
        let _ = c.add_state("walk", walk, true).expect("new state");
        let _ = c.add_state("idle", idle, true).expect("new state");
        c
    }

    #[test]
    fn first_state_is_initial() {
        let c = controller_pair();
        assert_eq!(c.current_state().as_str(), "walk");
        assert!(!c.is_transitioning());
    }

    #[test]
    fn duplicate_state_rejected() {
        let mut c = controller_pair();
        let again = slide_clip("walk2", 1.0, Vec3::ZERO, Vec3::ONE);
        assert_eq!(
            c.add_state("walk", again, false),
            Err(ControllerError::DuplicateState {
                state: "walk".to_owned()
            })
        );
    }

    #[test]
    fn looping_clip_wraps_playback() {
        let mut c = controller_pair();
        let mut pose = LocalPose::identity(1);
        for _ in 0..6 {
            c.update(0.25, &mut pose); // 1.5 s total, clip duration 1 s
        }
        assert!((c.play_time() - 0.5).abs() < EPS);
        // Walk goes 0 -> 2 over 1 s; phase 0.5 => x == 1.
        assert!((pose.get(0).translation.x - 1.0).abs() < 1e-3);
    }

    #[test]
    fn non_looping_clip_clamps_at_duration() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let run = slide_clip("run", 0.5, vec3(0.0, 0.0, 0.0), vec3(4.0, 0.0, 0.0));
        let run_id = c.add_state("run", run, false).expect("new state");
        c.add_transition(&walk, Transition::instant(run_id, vec![]))
            .expect("valid");
        let mut pose = LocalPose::identity(1);
        for _ in 0..10 {
            c.update(0.1, &mut pose);
        }
        assert_eq!(c.current_state().as_str(), "run");
        assert!((pose.get(0).translation.x - 4.0).abs() < 1e-4);
    }

    #[test]
    fn instant_transition_fires_on_condition() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let idle = StateId("idle".to_owned());
        c.add_parameter("moving", ParamValue::Flag(false))
            .expect("declared");
        c.add_transition(
            &walk,
            Transition::instant(
                idle,
                vec![Condition::Flag {
                    name: "moving".to_owned(),
                    is: false,
                }],
            ),
        )
        .expect("valid transition");
        let mut pose = LocalPose::identity(1);
        c.update(0.1, &mut pose); // moving == false => fires
        assert_eq!(c.current_state().as_str(), "idle");
    }

    #[test]
    fn condition_needs_parameter_to_hold() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let idle = StateId("idle".to_owned());
        c.add_parameter("moving", ParamValue::Flag(true))
            .expect("declared");
        c.add_transition(
            &walk,
            Transition::instant(
                idle,
                vec![Condition::Flag {
                    name: "moving".to_owned(),
                    is: false,
                }],
            ),
        )
        .expect("valid transition");
        let mut pose = LocalPose::identity(1);
        c.update(0.1, &mut pose); // moving == true => no fire
        assert_eq!(c.current_state().as_str(), "walk");
        c.set_parameter("moving", ParamValue::Flag(false))
            .expect("declared");
        c.update(0.1, &mut pose); // now it fires
        assert_eq!(c.current_state().as_str(), "idle");
    }

    #[test]
    fn crossfade_blends_both_clips_half_way() {
        let mut c = AnimationController::new(1);
        let walk = c
            .add_state(
                "walk",
                slide_clip("walk", 1.0, vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0)),
                true,
            )
            .expect("new state");
        let stand = c
            .add_state(
                "stand",
                slide_clip("stand", 1.0, vec3(10.0, 0.0, 0.0), vec3(10.0, 0.0, 0.0)),
                true,
            )
            .expect("new state");
        c.add_transition(&walk, Transition::crossfade_to(stand, 0.4, vec![]))
            .expect("valid");
        let mut pose = LocalPose::identity(1);
        c.update(0.2, &mut pose); // elapsed == 0.2 == half of 0.4
        assert!(c.is_transitioning());
        // Outgoing at t=0.2/1.0 => x = 0.4; incoming constant 10. Half blend = 5.2.
        assert!((pose.get(0).translation.x - 5.2).abs() < 1e-3);
    }

    #[test]
    fn crossfade_completes_and_switches() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let idle = StateId("idle".to_owned());
        c.add_transition(&walk, Transition::crossfade_to(idle, 0.3, vec![]))
            .expect("valid");
        let mut pose = LocalPose::identity(1);
        c.update(0.35, &mut pose); // elapsed 0.35 > duration 0.3
        assert!(!c.is_transitioning());
        assert_eq!(c.current_state().as_str(), "idle");
    }

    #[test]
    fn first_added_transition_wins() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let idle = StateId("idle".to_owned());
        c.add_parameter("trigger", ParamValue::Flag(true))
            .expect("declared");
        // Two valid instant transitions; the walk->idle one is added first.
        c.add_transition(
            &walk,
            Transition::instant(
                idle,
                vec![Condition::Flag {
                    name: "trigger".to_owned(),
                    is: true,
                }],
            ),
        )
        .expect("valid");
        // walk->walk is rejected, so second transition must target another
        // state; duplicate would also be walk->idle - use run.
        let run = slide_clip("run", 1.0, Vec3::ZERO, vec3(1.0, 0.0, 0.0));
        let run_id = c.add_state("run", run, true).expect("new state");
        c.add_transition(
            &walk,
            Transition::instant(
                run_id,
                vec![Condition::Flag {
                    name: "trigger".to_owned(),
                    is: true,
                }],
            ),
        )
        .expect("valid");
        let mut pose = LocalPose::identity(1);
        c.update(0.1, &mut pose);
        assert_eq!(c.current_state().as_str(), "idle");
    }

    #[test]
    fn validation_rejects_bad_transitions_and_params() {
        let mut c = controller_pair();
        let walk = StateId("walk".to_owned());
        let ghost = StateId("ghost".to_owned());

        assert_eq!(
            c.add_transition(&ghost, Transition::instant(walk.clone(), vec![])),
            Err(ControllerError::UnknownState {
                state: "ghost".to_owned()
            })
        );
        assert_eq!(
            c.add_transition(
                &walk,
                Transition::instant(StateId("ghost".to_owned()), vec![])
            ),
            Err(ControllerError::UnknownState {
                state: "ghost".to_owned()
            })
        );
        assert_eq!(
            c.add_transition(&walk, Transition::instant(walk.clone(), vec![])),
            Err(ControllerError::SelfTransition {
                state: "walk".to_owned()
            })
        );
        assert!(matches!(
            c.add_transition(
                &walk,
                Transition::crossfade_to(StateId("idle".to_owned()), -0.5, vec![])
            ),
            Err(ControllerError::InvalidDuration { duration: -0.5 })
        ));
        assert_eq!(
            c.add_transition(
                &walk,
                Transition::instant(
                    StateId("idle".to_owned()),
                    vec![Condition::Number {
                        name: "speed".to_owned(),
                        op: NumberOp::Greater,
                        value: 0.0,
                    }],
                )
            ),
            Err(ControllerError::UnknownParameter {
                parameter: "speed".to_owned()
            })
        );
    }

    #[test]
    fn number_conditions_compare_numerically() {
        let mut c = AnimationController::new(1);
        let a = c
            .add_state("a", slide_clip("a", 1.0, Vec3::ZERO, Vec3::ONE), true)
            .expect("new state");
        let b = c
            .add_state(
                "b",
                slide_clip("b", 1.0, Vec3::ZERO, vec3(2.0, 0.0, 0.0)),
                true,
            )
            .expect("new state");
        c.add_parameter("speed", ParamValue::Number(0.0))
            .expect("declared");
        c.add_transition(
            &a,
            Transition::instant(
                b,
                vec![Condition::Number {
                    name: "speed".to_owned(),
                    op: NumberOp::GreaterEq,
                    value: 1.0,
                }],
            ),
        )
        .expect("valid");
        let mut pose = LocalPose::identity(1);
        c.update(0.1, &mut pose);
        assert_eq!(c.current_state().as_str(), "a");
        c.set_parameter("speed", ParamValue::Number(1.0))
            .expect("declared");
        c.update(0.1, &mut pose);
        assert_eq!(c.current_state().as_str(), "b");
    }

    #[test]
    fn non_positive_or_nan_dt_changes_nothing() {
        let mut c = controller_pair();
        let mut pose = LocalPose::identity(1);
        c.update(0.25, &mut pose);
        let t = c.play_time();
        c.update(-1.0, &mut pose);
        c.update(f32::NAN, &mut pose);
        c.update(0.0, &mut pose);
        assert_eq!(c.play_time(), t);
    }

    #[test]
    fn same_sequence_replays_bit_identically() {
        let run_once = || {
            let mut c = controller_pair();
            let walk = StateId("walk".to_owned());
            let idle = StateId("idle".to_owned());
            c.add_parameter("moving", ParamValue::Flag(true))
                .expect("declared");
            c.add_transition(
                &walk,
                Transition::crossfade_to(
                    idle,
                    0.4,
                    vec![Condition::Flag {
                        name: "moving".to_owned(),
                        is: false,
                    }],
                ),
            )
            .expect("valid");
            let mut pose = LocalPose::identity(1);
            let mut seen = Vec::new();
            for step in 0..20 {
                if step == 8 {
                    c.set_parameter("moving", ParamValue::Flag(false))
                        .expect("declared");
                }
                c.update(0.05, &mut pose);
                seen.push(pose.get(0).translation.x.to_bits());
            }
            seen
        };
        let a = run_once();
        let b = run_once();
        assert_eq!(
            a, b,
            "same parameter/update sequence must replay identically"
        );
    }
}
