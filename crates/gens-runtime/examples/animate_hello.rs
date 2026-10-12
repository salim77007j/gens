//! Gens animation + AI + UI hello world (Phase 4).
//!
//! A patrol agent on a grid is driven by a behavior tree (gens-ai) whose
//! movement feeds an animation state machine (gens-animation) whose pose
//! drives an ECS entity through the Phase 1 propagation pipeline, while
//! an egui HUD (gens-ui) renders the agent state headlessly. Nothing
//! touches a GPU: the UI pass is pure CPU tessellation, hashed into the
//! evidence.
//!
//! ```text
//! BT decision -> steering velocity -> position integration
//!   -> speed parameter -> AnimationController -> FK -> ECS transform
//!   -> WorldTransform propagation -> egui HUD pass -> frame hash
//! ```
//!
//! Two identical runs must produce **bit-identical** trajectories
//! (positions, animation states and UI frame hashes) - the Phase 4
//! in-process determinism proof (see `docs/RISKS.md` R-013).
//!
//! ```text
//! cargo run -p gens-runtime --example animate_hello -- --steps 900 --out animate_hello.json
//! ```

use gens_ai::{
    Behavior, Blackboard, BlackboardValue, Connectivity, Grid, NodeStatus, PathFollower,
};
use gens_animation::{
    AnimationController, Clip, Condition as AnimCondition, JointTrack, LocalPose, ParamValue,
    QuatKey, QuatTrack, Skeleton, Transition as AnimTransition, VecKey, VecTrack,
};
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::game_loop::{GameLoop, LoopConfig};
use gens_core::logging::{info, init};
use gens_core::time::FixedSource;
use gens_ecs::{Active, EngineSchedules, Name, Transform, WorldTransform};
use gens_math::{Quat, Vec3, vec3};
use gens_ui::{UiContext, UiFrame, UiInput};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Patrol waypoints in cell coordinates (a rectangular loop).
const WAYPOINTS: [(u32, u32); 4] = [(1, 1), (10, 1), (10, 8), (1, 8)];
/// Agent speed in cells (world units) per second.
const SPEED: f32 = 2.0;
/// Distance at which the agent considers a waypoint reached.
const REACH_RADIUS: f32 = 0.35;
/// Seconds to stand still at each waypoint (the idle state window).
const DWELL_SECONDS: f32 = 1.0;

/// Shared mutable agent state the behavior tree acts on.
struct AgentRuntime {
    pos: Vec3,
    waypoint_idx: usize,
    path: Vec<Vec3>,
    follower: PathFollower,
    dwell_elapsed: f32,
}

impl AgentRuntime {
    fn new(grid: &Grid) -> Self {
        let (sx, sy) = WAYPOINTS[0];
        Self {
            pos: grid.cell_center(sx, sy),
            waypoint_idx: 0,
            path: vec![grid.cell_center(sx, sy)],
            follower: PathFollower::new(),
            dwell_elapsed: 0.0,
        }
    }

    fn target(&self, grid: &Grid) -> Vec3 {
        let (cx, cy) = WAYPOINTS[self.waypoint_idx];
        grid.cell_center(cx, cy)
    }

    fn plan_path(&mut self, grid: &Grid) {
        let (sx, sy) = grid.world_to_cell(self.pos);
        let (sx, sy) = (sx.max(0) as u32, sy.max(0) as u32);
        let goal = WAYPOINTS[self.waypoint_idx];
        // The agent stands on walkable cells by construction; if a
        // rounding boundary ever places it on a blocked cell, degrade to
        // a direct steer (honest fallback, no panic in the loop).
        self.path = if grid.walkable(sx, sy) {
            grid.find_path((sx, sy), goal, Connectivity::Four)
                .map(|cells| cells.iter().map(|&(x, y)| grid.cell_center(x, y)).collect())
                .unwrap_or_else(|_| vec![self.target(grid)])
        } else {
            vec![self.target(grid)]
        };
        self.follower.reset();
    }
}

fn set_flag(bb: &mut Blackboard, key: &str, value: bool) {
    bb.set(key, BlackboardValue::Flag(value));
}

/// The patrol behavior tree: dwell at waypoints, otherwise steer along
/// the planned path. Observations enter via the blackboard; the same
/// (blackboard, runtime, dt) sequence always produces the same actions.
fn build_brain(grid: Grid, runtime: Rc<RefCell<AgentRuntime>>) -> Box<dyn Behavior> {
    use gens_ai::bt::nodes::{Action, Condition, Selector, Sequence};

    // Each leaf closure owns its own clone of the shared state.
    let dwell_grid = grid.clone();
    let dwell_runtime = runtime.clone();
    let steer_grid = grid;
    let steer_runtime = runtime;
    Box::new(Selector::new(vec![
        // Branch 1: near the waypoint -> dwell, then advance the target.
        Box::new(Sequence::new(vec![
            Box::new(Condition::new(|bb: &Blackboard| {
                bb.flag("near_waypoint").unwrap_or(false)
            })),
            Box::new(Action::new(move |bb: &mut Blackboard, dt| {
                let mut agent = dwell_runtime.borrow_mut();
                agent.dwell_elapsed += dt;
                set_flag(bb, "moving", false);
                if agent.dwell_elapsed >= DWELL_SECONDS {
                    agent.dwell_elapsed = 0.0;
                    agent.waypoint_idx = (agent.waypoint_idx + 1) % WAYPOINTS.len();
                    agent.plan_path(&dwell_grid);
                    set_flag(bb, "near_waypoint", false);
                    NodeStatus::Success
                } else {
                    NodeStatus::Running
                }
            })),
        ])),
        // Branch 2: steer along the planned path.
        Box::new(Action::new(move |bb: &mut Blackboard, dt| {
            let mut agent = steer_runtime.borrow_mut();
            let target_cell = agent.target(&steer_grid);
            // Copy the inputs: the RefMut guard makes field borrows
            // overlap, so steer takes owned snapshots.
            let (path, pos) = (agent.path.clone(), agent.pos);
            let velocity = agent.follower.steer(&path, pos, SPEED, REACH_RADIUS);
            agent.pos += velocity * dt;
            if (target_cell - agent.pos).length() <= REACH_RADIUS {
                // Snap to the exact center: the arrival contract.
                agent.pos = target_cell;
                set_flag(bb, "near_waypoint", true);
                set_flag(bb, "moving", false);
                NodeStatus::Success
            } else {
                set_flag(bb, "near_waypoint", false);
                set_flag(bb, "moving", true);
                NodeStatus::Running
            }
        })),
    ]))
}

/// Two-joint skeleton: root (carries patrol motion) + bob (bounce).
fn skeleton() -> Skeleton {
    Skeleton::new(&["root", "bob"], &[None, Some(0)]).expect("valid hierarchy")
}

/// Walk clip: the bob joint oscillates vertically, the root sways.
fn walk_clip() -> Clip {
    let mut tracks = BTreeMap::new();
    tracks.insert(
        1,
        JointTrack {
            translation: Some(
                VecTrack::new(
                    vec![
                        VecKey {
                            time: 0.0,
                            value: vec3(0.0, 0.0, 0.0),
                        },
                        VecKey {
                            time: 0.25,
                            value: vec3(0.0, 0.18, 0.0),
                        },
                        VecKey {
                            time: 0.5,
                            value: vec3(0.0, 0.0, 0.0),
                        },
                        VecKey {
                            time: 0.75,
                            value: vec3(0.0, 0.18, 0.0),
                        },
                        VecKey {
                            time: 1.0,
                            value: vec3(0.0, 0.0, 0.0),
                        },
                    ],
                    gens_animation::Interpolation::Linear,
                )
                .expect("valid track"),
            ),
            ..JointTrack::default()
        },
    );
    tracks.insert(
        0,
        JointTrack {
            rotation: Some(
                QuatTrack::new(
                    vec![
                        QuatKey {
                            time: 0.0,
                            value: Quat::IDENTITY,
                        },
                        QuatKey {
                            time: 0.25,
                            value: Quat::from_rotation_z(0.05),
                        },
                        QuatKey {
                            time: 0.75,
                            value: Quat::from_rotation_z(-0.05),
                        },
                        QuatKey {
                            time: 1.0,
                            value: Quat::IDENTITY,
                        },
                    ],
                    gens_animation::Interpolation::Linear,
                )
                .expect("valid track"),
            ),
            ..JointTrack::default()
        },
    );
    Clip::new("walk", 1.0, tracks).expect("valid clip")
}

/// Idle clip: a slow, shallow breathing bob.
fn idle_clip() -> Clip {
    Clip::translation(
        "idle",
        2.0,
        1,
        vec![
            VecKey {
                time: 0.0,
                value: vec3(0.0, 0.0, 0.0),
            },
            VecKey {
                time: 1.0,
                value: vec3(0.0, 0.05, 0.0),
            },
            VecKey {
                time: 2.0,
                value: vec3(0.0, 0.0, 0.0),
            },
        ],
    )
    .expect("valid clip")
}

fn build_grid() -> Grid {
    let mut grid = Grid::new(12, 10);
    // A central obstacle the corridor routes around.
    for (x, y) in [(5, 4), (6, 4), (5, 5), (6, 5), (5, 6), (6, 6)] {
        grid.set_walkable(x, y, false);
    }
    grid
}

/// One sampled trajectory point (bit-identical across replays).
type Sample = (u32, f32, f32, &'static str, u64);

/// FNV-1a over the frame's mesh geometry - the UI content hash.
fn fnv_frame(frame: &UiFrame) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    fn mix(bytes: &[u8], hash: &mut u64) {
        for b in bytes {
            *hash ^= u64::from(*b);
            *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn by(f: f32) -> [u8; 4] {
        f.to_bits().to_le_bytes()
    }
    mix(&(frame.meshes.len() as u32).to_le_bytes(), &mut hash);
    for mesh in &frame.meshes {
        mix(&(mesh.indices.len() as u32).to_le_bytes(), &mut hash);
        for v in &mesh.vertices {
            mix(&by(v.pos[0]), &mut hash);
            mix(&by(v.pos[1]), &mut hash);
            mix(&by(v.color[3]), &mut hash);
        }
    }
    hash
}

fn state_label(state: &str) -> &'static str {
    if state == "walk" { "walk" } else { "idle" }
}

fn run_simulation(steps: u32) -> anyhow::Result<Vec<Sample>> {
    let grid = build_grid();
    let runtime = Rc::new(RefCell::new(AgentRuntime::new(&grid)));
    runtime.borrow_mut().plan_path(&grid);
    let mut tree = build_brain(grid.clone(), runtime.clone());
    let mut blackboard = Blackboard::new();
    blackboard.set("near_waypoint", BlackboardValue::Flag(false));

    let skel = skeleton();
    let mut controller = AnimationController::new(skel.joint_count());
    let walk = controller
        .add_state("walk", walk_clip(), true)
        .expect("state");
    let idle = controller
        .add_state("idle", idle_clip(), true)
        .expect("state");
    controller
        .add_parameter("moving", ParamValue::Flag(true))
        .expect("declared");
    controller
        .add_transition(
            &walk,
            AnimTransition::crossfade_to(
                idle.clone(),
                0.2,
                vec![AnimCondition::Flag {
                    name: "moving".to_owned(),
                    is: false,
                }],
            ),
        )
        .expect("valid");
    controller
        .add_transition(
            &idle,
            AnimTransition::crossfade_to(
                walk,
                0.2,
                vec![AnimCondition::Flag {
                    name: "moving".to_owned(),
                    is: true,
                }],
            ),
        )
        .expect("valid");

    let mut ui = UiContext::new();
    let mut pose = LocalPose::identity(skel.joint_count());

    let mut world = gens_ecs::ecs::world::World::new();
    let agent = world
        .spawn((Name::new("patrol_agent"), Transform::IDENTITY, Active))
        .id();
    let mut schedules = EngineSchedules::default();

    let source = FixedSource::new(Duration::from_secs_f64(1.0 / 60.0));
    let mut game_loop = GameLoop::new(LoopConfig::default(), source);
    let mut samples = Vec::new();

    for step in 1..=steps {
        game_loop.run_frame(|_| {
            // 1. Observations -> blackboard.
            let near = {
                let agent_state = runtime.borrow();
                (agent_state.target(&grid) - agent_state.pos).length() <= REACH_RADIUS
            };
            blackboard.set("near_waypoint", BlackboardValue::Flag(near));

            // 2. The behavior tree decides; actions move the agent.
            tree.tick(&mut blackboard, 1.0 / 60.0);

            // 3. Animation follows the movement flag; FK -> ECS transform.
            let moving = blackboard.flag("moving").unwrap_or(false);
            controller
                .set_parameter("moving", ParamValue::Flag(moving))
                .expect("parameter declared");
            controller.update(1.0 / 60.0, &mut pose);
            let agent_pos = runtime.borrow().pos;
            let root_global = pose.global_matrix(&skel, 0);
            let offset = root_global.w_axis.truncate();
            world
                .get_mut::<Transform>(agent)
                .expect("agent alive")
                .position = vec3(agent_pos.x, offset.y, agent_pos.z);

            // 4. WorldTransform propagation (Phase 1 pipeline).
            schedules.run_frame(&mut world);
        });

        // 5. Headless UI pass on the propagated state; frame hashed.
        let wt = world.get::<WorldTransform>(agent).expect("propagation ran");
        let p = wt.position();
        let state_name = controller.current_state().as_str().to_owned();
        let wp = runtime.borrow().waypoint_idx;
        let frame = ui.run(
            UiInput {
                time_seconds: f64::from(step) / 60.0,
                dt_seconds: 1.0 / 60.0,
                size_px: [320, 90],
                events: Vec::new(),
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label(format!("agent  x={:.3} z={:.3}", p.x, p.z));
                    ui.label(format!(
                        "state  {}  waypoint {wp}",
                        state_label(&state_name)
                    ));
                });
            },
        );
        if step % 20 == 0 || step == steps {
            samples.push((step, p.x, p.z, state_label(&state_name), fnv_frame(&frame)));
        }
    }
    Ok(samples)
}

fn main() -> anyhow::Result<()> {
    let mut steps: u32 = 900;
    let mut out = PathBuf::from("animate_hello.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--steps" => {
                steps = args
                    .next()
                    .context("--steps needs a value")?
                    .parse()
                    .context("--steps must be a number")?;
            }
            "--out" => {
                out = PathBuf::from(args.next().context("--out needs a value")?);
            }
            other => anyhow::bail!("unknown argument: {other} (use --steps N, --out PATH)"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - animation + AI + UI hello world");
    info!(steps, ?out, %filter, "run configuration");

    // --- Two identical runs must be bit-identical (R-013, in-process) ------
    let run_a = run_simulation(steps)?;
    let run_b = run_simulation(steps)?;
    assert_eq!(run_a, run_b, "two runs must be bit-identical");
    info!(samples = run_a.len(), "determinism: two runs bit-identical");

    // --- Behavioral contract checks -----------------------------------------
    // 1) The patrol spans the map horizontally.
    let min_x = run_a
        .iter()
        .map(|&(_, x, _, _, _)| x)
        .fold(f32::INFINITY, f32::min);
    let max_x = run_a
        .iter()
        .map(|&(_, x, _, _, _)| x)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        max_x - min_x > 8.0,
        "patrol must span the map: x range [{min_x}, {max_x}]"
    );

    // 2) Both animation states are observed.
    let mut saw_walk = false;
    let mut saw_idle = false;
    for &(_, _, _, state, _) in &run_a {
        saw_walk |= state == "walk";
        saw_idle |= state == "idle";
    }
    assert!(saw_walk, "walk state must appear during movement");
    assert!(saw_idle, "idle state must appear during dwell");

    // 3) A meaningful path was walked across sampled segments.
    let mut walked_distance = 0.0f32;
    for pair in run_a.windows(2) {
        let (_, x0, z0, _, _) = pair[0];
        let (_, x1, z1, _, _) = pair[1];
        walked_distance += ((x1 - x0).powi(2) + (z1 - z0).powi(2)).sqrt();
    }
    assert!(
        walked_distance > 16.0,
        "agent must have walked a meaningful distance: {walked_distance}"
    );

    info!(
        min_x,
        max_x,
        walked_distance,
        samples = run_a.len(),
        "animate hello verification passed"
    );

    // --- Write the trajectory as JSON evidence -------------------------------
    let mut json = String::from("{\"trajectory\": [\n");
    for (i, &(s, x, z, state, ui_hash)) in run_a.iter().enumerate() {
        let comma = if i + 1 == run_a.len() { "" } else { "," };
        json.push_str(&format!(
            "  {{\"step\": {s}, \"x\": {x}, \"z\": {z}, \"state\": \"{state}\", \"ui_hash\": {ui_hash}}}{comma}\n"
        ));
    }
    json.push_str("]}\n");
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, json).with_context(|| format!("writing {out:?}"))?;
    info!(
        ?out,
        bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        "trajectory evidence written"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_frames_hash_stably_for_identical_content() {
        let mut ui = UiContext::new();
        let build = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("hash me");
            });
        };
        let a = ui.run(UiInput::new([200, 80], 0.0, 0.016), build);
        let b = ui.run(UiInput::new([200, 80], 0.1, 0.016), build);
        assert_eq!(fnv_frame(&a), fnv_frame(&b));
    }

    #[test]
    fn grid_has_a_blocked_center_the_path_routes_around() {
        let grid = build_grid();
        assert!(!grid.walkable(5, 5));
        let path = grid
            .find_path((1, 1), (10, 8), Connectivity::Four)
            .expect("waypoints connected");
        for &(x, y) in &path {
            assert!(grid.walkable(x, y), "path must stay on walkable cells");
        }
    }
}
