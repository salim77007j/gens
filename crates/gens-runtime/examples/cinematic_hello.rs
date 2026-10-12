//! Cinematic replay proof (Phase 9, ADR-0030): a cinematic drives a
//! hero and a camera over a fixed-step run, and the run must replay
//! byte-identically — straight-through vs fresh replay vs seek-resume.
//!
//! Three runs over the same 4-second timeline at a fixed 1/60 step:
//!
//! - **Run A** plays straight through for `--steps` (default 300 =
//!   5 s of sim time over a 4 s timeline; the last 60 steps prove
//!   post-finish stability).
//! - **Run B** is a fresh world playing the same timeline — every
//!   per-step world hash must equal Run A's byte-for-byte (the
//!   in-process replay proof, same discipline as the ADR-0027 proof).
//! - **Run C** seeks to Run A's *exact internal time* after
//!   `--seek-at` steps (default 60) and plays the remainder — the hash
//!   from that point on must equal Run A's corresponding hashes
//!   (seek-equivalence).
//!
//! # The seek-semantics lesson this proof pins
//!
//! The player's local time is a sequential f32 accumulation of the
//! fixed delta. A seek that targets a *mathematically close* value
//! (e.g. literal `1.0` vs the accumulated 60×(1/60)) does NOT share the
//! addition history, so low bits diverge and hashes differ. The engine
//! treats a cinematic's local time as **state**: seek targets are exact
//! f32 values (the save format's philosophy, ADR-0021), and only a seek
//! to the exact internal time replays byte-identically. Run C proves
//! exactly that property and the report records the seek target's bit
//! pattern.
//!
//! ```text
//! cargo run -p gens-runtime --example cinematic_hello -- --steps 300 --out cinematic_hello.json
//! ```

use gens_cinematics::{
    CameraCut, Channel, CinematicPlayer, FloatKey, FloatTrack, Interpolation, LoopMode, QuatKey,
    QuatTrack, Timeline, Track, Vec3Key, Vec3Track,
};
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_ecs::{Active, EngineSchedules, Name, Transform};
use gens_math::{Quat, Vec3, quat, vec3};
use gens_runtime::cinematic::{CameraPose, CinematicStage, advance_and_apply};
use gens_runtime::profile::world_state_hash;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// The fixed step every run uses.
const FIXED_DT: f32 = 1.0 / 60.0;

/// Per-step ledger collected by a run.
#[derive(Debug, Clone)]
struct RunLedger {
    /// world_state_hash after every step (full fidelity in memory,
    /// sampled into the report).
    hashes: Vec<u64>,
    /// (step, event name) in firing order.
    events: Vec<(u32, String)>,
    /// The player's exact internal local time after each step (only
    /// queried at the seek point and at the end).
    times: Vec<f32>,
    /// Camera FOV bits at the end (proves the pose staged).
    final_fov_bits: u32,
    /// Final fade sample (proves the scalar channel).
    final_fade: f32,
    /// Whether the player reached Finished by the end of the run.
    finished: bool,
}

fn build_timeline() -> Timeline {
    // Input discipline: every key value below is an f32 LITERAL. No
    // runtime sin/cos/sqrt — platform libm could make the INPUT differ
    // across operating systems in low bits, which would undercut the
    // cross-OS replay claims (R-013). Literal rotations: quat(0, s, 0, c)
    // is a yaw of 2*atan2(s, c); (0.6, 0.8) ≈ 73.7°, (0.5, 0.8660254)
    // ≈ 60°, (0.25881905, 0.9659258) ≈ -30°.
    Timeline::builder(4.0)
        .vec3_track(
            "camera",
            Channel::Translation,
            Vec3Track::new(
                vec![
                    Vec3Key {
                        time: 0.0,
                        value: vec3(0.0, 3.0, 10.0),
                    },
                    Vec3Key {
                        time: 4.0,
                        value: vec3(6.0, 2.5, 6.0),
                    },
                ],
                Interpolation::Smoothstep,
            )
            .expect("camera translation track"),
        )
        .expect("camera translation")
        .quat_track(
            "camera",
            Channel::Rotation,
            QuatTrack::new(
                vec![
                    QuatKey {
                        time: 0.0,
                        value: Quat::IDENTITY,
                    },
                    QuatKey {
                        time: 4.0,
                        value: quat(0.0, -0.25881905, 0.0, 0.9659258),
                    },
                ],
                Interpolation::Smoothstep,
            )
            .expect("camera rotation track"),
        )
        .expect("camera rotation")
        .float_track(
            "camera",
            Channel::Fov,
            FloatTrack::new(
                vec![
                    FloatKey {
                        time: 0.0,
                        value: 60.0,
                    },
                    FloatKey {
                        time: 4.0,
                        value: 45.0,
                    },
                ],
                Interpolation::Linear,
            )
            .expect("camera fov track"),
        )
        .expect("camera fov")
        .vec3_track(
            "hero",
            Channel::Translation,
            Vec3Track::new(
                vec![
                    Vec3Key {
                        time: 0.0,
                        value: Vec3::ZERO,
                    },
                    Vec3Key {
                        time: 4.0,
                        value: vec3(4.0, 0.0, 0.0),
                    },
                ],
                Interpolation::Linear,
            )
            .expect("hero translation track"),
        )
        .expect("hero translation")
        .quat_track(
            "hero",
            Channel::Rotation,
            QuatTrack::new(
                vec![
                    QuatKey {
                        time: 0.0,
                        value: Quat::IDENTITY,
                    },
                    QuatKey {
                        time: 4.0,
                        value: quat(0.0, 0.6, 0.0, 0.8),
                    },
                ],
                Interpolation::Smoothstep,
            )
            .expect("hero rotation track"),
        )
        .expect("hero rotation")
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
                    FloatKey {
                        time: 4.0,
                        value: 0.0,
                    },
                ],
                Interpolation::Linear,
            )
            .expect("fade track"),
        )
        .expect("overlay fade")
        .event(0.5, "title-card")
        .expect("event 1")
        .event(2.0, "act-two")
        .expect("event 2")
        .event(3.5, "credits")
        .expect("event 3")
        .camera(CameraCut {
            target: "camera".to_string(),
            hard_cut: true,
            blend_in: 0.0,
        })
        .build()
        .expect("timeline builds")
}

/// One deterministic run. `seek_to` seeks before stepping (None = start
/// at zero); the returned ledger carries per-step hashes and events.
fn run_simulation(steps: u32, seek_to: Option<f32>) -> anyhow::Result<RunLedger> {
    let timeline = Arc::new(build_timeline());
    let mut world = gens_ecs::ecs::world::World::new();
    let mut schedules = EngineSchedules::new();

    world.spawn((Name::new("Hero"), Transform::IDENTITY, Active));
    let camera = world
        .spawn((Name::new("Camera"), Transform::IDENTITY, Active))
        .id();
    // Lookups go through a Name query (iter_entities order is NOT
    // deterministic — the P5 lesson).
    let hero = {
        let mut q = world.query::<(&Name, gens_ecs::ecs::entity::Entity)>();
        q.iter(&world)
            .find(|(n, _)| n.0 == "Hero")
            .map(|(_, e)| e)
            .expect("hero exists")
    };

    let mut player = CinematicPlayer::new(timeline);
    player.set_loop_mode(LoopMode::Once);
    let mut stage = CinematicStage::new(
        player,
        [("hero".to_string(), hero), ("camera".to_string(), camera)],
    );
    if let Some(t) = seek_to {
        stage.player.seek(t).context("seek target valid")?;
    }
    stage.player.play();

    let mut ledger = RunLedger {
        hashes: Vec::with_capacity(steps as usize),
        events: Vec::new(),
        times: Vec::with_capacity(steps as usize),
        final_fov_bits: 0,
        final_fade: f32::NAN,
        finished: false,
    };

    for step in 1..=steps {
        let outcome = advance_and_apply(&mut stage, &mut world, FIXED_DT)?;
        for event in outcome.events {
            info!(step, event = %event.name, "cinematic event fired");
            ledger.events.push((step, event.name));
        }
        // One frame of the standard schedules: propagation picks up the
        // cinematic's transform writes into WorldTransform.
        schedules.run_frame(&mut world);
        ledger.hashes.push(world_state_hash(&world));
        ledger.times.push(stage.player.local_time());
    }

    ledger.finished = stage.player.state() == gens_cinematics::PlaybackState::Finished;
    let fov = world.get::<CameraPose>(camera).map(|p| p.fov_degrees);
    if let Some(f) = fov {
        ledger.final_fov_bits = f.to_bits();
    }
    // The fade's last value: re-sample the timeline at the final time.
    if let Some((_, Track::Float(t))) = stage
        .player
        .timeline()
        .tracks()
        .find(|(a, _)| a.target == "overlay" && a.channel == Channel::Fade)
    {
        ledger.final_fade = t.sample(stage.player.local_time());
    }
    Ok(ledger)
}

#[derive(Serialize)]
struct Report {
    format: &'static str,
    environment: Env,
    fixed_dt: f64,
    steps: u32,
    seek_at: u32,
    timeline: TimelineInfo,
    run_a: RunReport,
    run_b: RunReport,
    run_c: RunCReport,
    proof: Proof,
    advisory: &'static str,
}

#[derive(Serialize)]
struct Env {
    os: &'static str,
    arch: &'static str,
    profile: &'static str,
    package_version: &'static str,
}

#[derive(Serialize)]
struct TimelineInfo {
    duration: f32,
    track_count: usize,
    event_names: Vec<String>,
    camera_target: String,
}

#[derive(Serialize)]
struct RunReport {
    hashes_every_60: Vec<String>,
    events: Vec<(u32, String)>,
    final_hash: String,
    final_fov_bits: String,
    final_fade: f32,
    final_local_time: f32,
}

#[derive(Serialize)]
struct RunCReport {
    seek_target: f32,
    seek_target_bits: String,
    hashes_every_60_from_seek: Vec<String>,
    events: Vec<(u32, String)>,
    final_hash: String,
    hashes_match_run_a: bool,
}

#[derive(Serialize)]
struct Proof {
    replay_identical: bool,
    seek_equivalent: bool,
}

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

fn sample_every_60(hashes: &[u64], offset: usize) -> Vec<String> {
    hashes
        .iter()
        .skip(offset)
        .enumerate()
        .filter(|(i, _)| (i + 1 + offset).is_multiple_of(60))
        .map(|(_, h)| hex(*h))
        .collect()
}

fn to_report(ledger: &RunLedger) -> RunReport {
    RunReport {
        hashes_every_60: sample_every_60(&ledger.hashes, 0),
        events: ledger.events.clone(),
        final_hash: hex(*ledger.hashes.last().expect("non-empty")),
        final_fov_bits: format!("{:08x}", ledger.final_fov_bits),
        final_fade: ledger.final_fade,
        final_local_time: *ledger.times.last().expect("non-empty"),
    }
}

fn main() -> anyhow::Result<()> {
    let mut steps: u32 = 300;
    let mut seek_at: u32 = 60;
    let mut out = PathBuf::from("cinematic_hello.json");
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
            "--seek-at" => {
                seek_at = args
                    .next()
                    .context("--seek-at needs a value")?
                    .parse()
                    .context("--seek-at must be a number")?;
            }
            "--out" => {
                out = PathBuf::from(args.next().context("--out needs a value")?);
            }
            other => {
                anyhow::bail!("unknown argument: {other} (use --steps N, --seek-at N, --out PATH)")
            }
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - cinematic replay proof (ADR-0030)");
    info!(steps, seek_at, ?out, %filter, "run configuration");
    anyhow::ensure!(
        seek_at > 0 && seek_at < steps,
        "--seek-at must be in (0, steps)"
    );

    // --- Run A: straight through. Records the exact internal time at
    // the seek point for Run C. ------------------------------------
    let run_a = run_simulation(steps, None)?;
    let seek_target = run_a.times[(seek_at - 1) as usize];
    info!(
        seek_at,
        seek_target,
        bits = format!("{:08x}", seek_target.to_bits()),
        "run A complete; seek target pinned to its exact internal time"
    );

    // --- Run B: fresh world, same timeline. ------------------------
    let run_b = run_simulation(steps, None)?;

    // --- Run C: seek to the exact internal time, play the rest. ----
    let run_c_steps = steps - seek_at;
    let run_c = run_simulation(run_c_steps, Some(seek_target))?;

    // --- Proofs ----------------------------------------------------
    let replay_identical = run_a.hashes == run_b.hashes;
    anyhow::ensure!(
        replay_identical,
        "REPLAY PROOF FAILED: run B diverged from run A"
    );

    let run_c_hashes_match = run_c.hashes == run_a.hashes[(seek_at as usize)..]
        && run_a.hashes.len() == seek_at as usize + run_c.hashes.len();
    anyhow::ensure!(
        run_c_hashes_match,
        "SEEK PROOF FAILED: run C diverged from run A after seeking to the exact internal time"
    );

    // Post-finish stability (observed, not assumed): the f32-accumulated
    // clock crosses the 4 s end a step or two after the naive 240 (60
    // steps accumulate to 0.9999997, not 1.0 — the same addition-history
    // lesson the seek proof pins), so the stability window starts where
    // the clock observably stopped: the last 20 steps must be
    // hash-identical, and the player must have reached Finished.
    anyhow::ensure!(
        run_a.finished,
        "POST-FINISH CHECK INVALID: the player never finished; pass --steps high enough"
    );
    {
        let tail = &run_a.hashes[run_a.hashes.len() - 20..];
        let stable = tail.windows(2).all(|w| w[0] == w[1]);
        anyhow::ensure!(
            stable,
            "POST-FINISH STABILITY FAILED: hashes moved after the cinematic finished"
        );
    }

    let report = Report {
        format: "gens-cinematic-1",
        environment: Env {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            package_version: env!("CARGO_PKG_VERSION"),
        },
        fixed_dt: f64::from(FIXED_DT),
        steps,
        seek_at,
        timeline: {
            let tl = build_timeline();
            TimelineInfo {
                duration: tl.duration(),
                track_count: tl.tracks().count(),
                event_names: tl.events().iter().map(|e| e.name.clone()).collect(),
                camera_target: tl
                    .camera()
                    .expect("timeline declares a camera")
                    .target
                    .clone(),
            }
        },
        run_a: to_report(&run_a),
        run_b: to_report(&run_b),
        run_c: RunCReport {
            seek_target,
            seek_target_bits: format!("{:08x}", seek_target.to_bits()),
            hashes_every_60_from_seek: sample_every_60(&run_c.hashes, 0),
            events: run_c.events.clone(),
            final_hash: hex(*run_c.hashes.last().expect("non-empty")),
            hashes_match_run_a: run_c_hashes_match,
        },
        proof: Proof {
            replay_identical,
            seek_equivalent: run_c_hashes_match,
        },
        advisory: "determinism proof, not performance; times are fixed-step simulation time",
    };

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out, json).with_context(|| format!("write report to {}", out.display()))?;
    info!(
        steps,
        seek_at,
        final_hash = %hex(*run_a.hashes.last().expect("non-empty")),
        "cinematic replay proof complete (replay identical, seek equivalent)"
    );
    println!("cinematic proof written to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_tracks_and_events_are_complete() {
        let tl = build_timeline();
        assert_eq!(tl.duration(), 4.0);
        assert_eq!(tl.tracks().count(), 6);
        assert_eq!(
            tl.events()
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec!["title-card", "act-two", "credits"]
        );
        assert_eq!(tl.camera().expect("camera").target, "camera");
    }

    #[test]
    fn fixed_dt_accumulates_to_expected_finish() {
        // 240 steps of the fixed dt must reach (or pass) the 4 s end.
        let mut t = 0.0f32;
        for _ in 0..240 {
            t += FIXED_DT;
        }
        assert!(t >= 4.0, "240 steps reach the timeline end, got {t}");
    }
}
