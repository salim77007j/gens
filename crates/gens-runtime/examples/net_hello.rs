//! Gens networking + data assets hello world (Phase 6).
//!
//! One scripted patrol agent whose scene is a **data document** (the
//! Phase 6 content format) and whose mid-run world state travels over
//! **real QUIC on loopback** (the Phase 6 transport + session stack):
//!
//! ```text
//! scenes/net-demo.json (embedded DataDocument)
//!   -> registry import + apply_data_document             [gens-assets]
//!      -> scripted agent (sandboxed Rhai, "patrol")      [gens-scripting]
//! HOST: simulate steps 1..=K -> capture_world -> Snapshot over QUIC
//!   -> session envelope (tick + FNV payload hash)         [gens-networking]
//! CLIENT: recv -> apply_save into a FRESH world -> continue K+1..=N
//!   -> final captures BYTE-IDENTICAL to the host's        [gens-runtime]
//! ```
//!
//! The property under test is the Phase 5 save/load continuation, now
//! across the wire: the client's continued run ends bit-identical to
//! the host's uninterrupted one. What crosses the network is hashes
//! and canonical documents — cross-platform float bit-identity is not
//! assumed (R-013).
//!
//! ```text
//! cargo run -p gens-runtime --example net_hello -- --steps 600 --replicate-at 300 --out net_hello.json
//! ```

use gens_core::anyhow::{self, Context};
use gens_core::logging::info;
use gens_ecs::{EngineSchedules, Name};
use gens_networking::{
    LOOPBACK_SERVER_NAME, LoopbackCert, QuicEndpoint, SESSION_PROTOCOL_VERSION, Session,
};
use gens_runtime::data::apply_data_document;
use gens_runtime::net::{recv_world_snapshot, send_world_snapshot};
use gens_runtime::save::{SaveWorld, apply_save, capture_world, save_json};
use gens_scripting::{ScriptHost, run_scripts};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// The scene as content: a scripted patrol agent + a static landmark.
const SCENE_DOC: &str = r#"{
    "format_version": 1,
    "name": "net-demo",
    "entities": [
        {
            "name": "patrol_agent",
            "translation": [1.0, 0.0, 1.0],
            "script": {
                "script": "patrol",
                "entry": "update",
                "vars": {
                    "wp": {"kind": "number", "value": 0.0},
                    "dwell": {"kind": "number", "value": 0.0},
                    "ticks": {"kind": "number", "value": 0.0},
                    "moving": {"kind": "flag", "value": false}
                }
            }
        },
        {
            "name": "landmark",
            "translation": [9.0, 2.0, 7.0],
            "tag": "landmark"
        }
    ]
}"#;

/// The whole decision layer, as content (same sandbox surface as the
/// Phase 5 example: pos_x/pos_z/set_pos, var_*/set_var_*, dt, math).
const PATROL_SCRIPT: &str = r#"
    fn update() {
        let speed = 1.5;
        let reach = 0.15;
        let dwell = 0.5;

        let px = pos_x();
        let pz = pos_z();
        let idx = var_num("wp");
        let i = to_int(idx);

        let wpx = [1.0, 9.0, 9.0, 1.0];
        let wpz = [1.0, 1.0, 7.0, 7.0];

        let tx = wpx[i];
        let tz = wpz[i];
        let dx = tx - px;
        let dz = tz - pz;
        let dist = sqrt(dx * dx + dz * dz);

        if dist < reach {
            set_var_flag("moving", false);
            let t = var_num("dwell") + dt();
            set_var_num("dwell", t);
            if t >= dwell {
                set_var_num("dwell", 0.0);
                set_var_num("wp", (idx + 1.0) % 4.0);
            }
        } else {
            set_var_flag("moving", true);
            let step = min(speed * dt(), dist);
            set_pos(px + dx / dist * step, 0.0, pz + dz / dist * step);
        }

        set_var_num("ticks", var_num("ticks") + 1.0);
    }
"#;

const DT: f64 = 1.0 / 60.0;

/// One side's simulation: world + schedules. Identity of the agent is
/// re-derived by name after every restore (bevy 0.16+ resource
/// entities occupy slots; the P5-6 lesson).
struct Sim {
    world: gens_ecs::ecs::world::World,
    schedules: EngineSchedules,
}

impl Sim {
    /// Host side: the scene document becomes the world (data pillar).
    fn from_document() -> anyhow::Result<(Sim, u64, usize)> {
        let mut registry = gens_assets::AssetRegistry::new();
        let (doc_id, doc) = gens_assets::import_data_document(
            &mut registry,
            "scenes/net-demo.json",
            SCENE_DOC.as_bytes(),
        )
        .context("scene import")?;
        let entry = registry.entry(doc_id).expect("registered");
        let (doc_hash, doc_size) = (entry.content_hash, entry.size);

        let mut world = gens_ecs::ecs::world::World::new();
        let mut host = ScriptHost::new();
        host.register("patrol", PATROL_SCRIPT)
            .context("script compile")?;
        world.insert_resource(host);
        apply_data_document(&mut world, &doc).context("data bridge")?;
        Ok((
            Sim {
                world,
                schedules: EngineSchedules::default(),
            },
            doc_hash,
            doc_size,
        ))
    }

    /// Client side: an empty stage + the same scripts; state arrives
    /// over the wire.
    fn empty_stage() -> anyhow::Result<Sim> {
        let mut world = gens_ecs::ecs::world::World::new();
        let mut host = ScriptHost::new();
        host.register("patrol", PATROL_SCRIPT)
            .context("script compile")?;
        world.insert_resource(host);
        Ok(Sim {
            world,
            schedules: EngineSchedules::default(),
        })
    }

    fn agent(&self) -> gens_ecs::ecs::prelude::Entity {
        self.world
            .iter_entities()
            .find_map(|r| {
                r.get::<Name>()
                    .filter(|n| n.as_str() == "patrol_agent")
                    .map(|_| r.id())
            })
            .expect("patrol agent present")
    }

    fn step(&mut self, step: u32) {
        run_scripts(&mut self.world, DT, f64::from(step) * DT);
        self.schedules.run_frame(&mut self.world);
    }
}

/// Everything the evidence JSON carries.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Evidence {
    protocol_version: u16,
    session_id: u64,
    scene_key: String,
    scene_hash: u64,
    scene_bytes: usize,
    steps: u32,
    replicate_at: u32,
    snapshot_bytes: usize,
    host_final: String,
    client_final: String,
    identical: bool,
    client_ticks: f64,
    agent_x: f32,
}

fn main() -> anyhow::Result<()> {
    gens_core::logging::init().expect("logging init");
    let args: Vec<String> = std::env::args().collect();
    let mut steps = 600_u32;
    let mut replicate_at = 300_u32;
    let mut out = PathBuf::from("net_hello.json");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--steps" => {
                i += 1;
                steps = args
                    .get(i)
                    .context("--steps needs a value")?
                    .parse()
                    .context("steps")?;
            }
            "--replicate-at" => {
                i += 1;
                replicate_at = args
                    .get(i)
                    .context("--replicate-at needs a value")?
                    .parse()
                    .context("replicate-at")?;
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(args.get(i).context("--out needs a value")?);
            }
            other => anyhow::bail!("unknown argument {other:?}"),
        }
        i += 1;
    }
    anyhow::ensure!(
        replicate_at > 0 && replicate_at < steps,
        "--replicate-at must be inside 1..steps"
    );
    let t = Duration::from_secs(30);

    let cert = LoopbackCert::generate().context("loopback cert")?;
    let server = QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert)
        .context("server bind")?;
    let addr = server.local_addr();

    // CLIENT thread: connects, receives the mid-run snapshot, continues.
    let client = std::thread::spawn(move || -> anyhow::Result<String> {
        let endpoint = QuicEndpoint::client(&cert).context("client bind")?;
        let link = endpoint
            .connect(addr, LOOPBACK_SERVER_NAME, t)
            .context("connect")?;
        let mut session = Session::client(link, t).context("client handshake")?;
        info!("client: session {} established", session.session_id);

        let (tick, save) = recv_world_snapshot(&mut session, t).context("snapshot recv")?;
        info!("client: snapshot for tick {tick} received");
        anyhow::ensure!(tick == u64::from(replicate_at), "snapshot tick mismatch");

        let mut sim = Sim::empty_stage()?;
        apply_save(&mut sim.world, &save).context("apply replicated snapshot")?;
        sim.schedules.run_frame(&mut sim.world); // derive WorldTransform

        for step in (replicate_at + 1)..=steps {
            sim.step(step);
        }

        // Continuity proof: the script's tick counter survived the wire
        // and kept counting exactly in step with the simulation.
        let agent = sim.agent();
        let sc = sim
            .world
            .get::<gens_scripting::ScriptComponent>(agent)
            .expect("agent scripted");
        let ticks = sc.vars.get_number("ticks").expect("ticks var");

        // The final state, canonicalized the same way the host will.
        let final_json = save_json(&capture_world(&sim.world)).context("client capture")?;
        session.close(0, "done");
        Ok(format!("TICKS:{ticks}\n{final_json}"))
    });

    // HOST: scene -> world -> accept session -> simulate -> replicate
    // at K -> continue. The accept happens first: the client thread is
    // already connecting.
    let (mut host_sim, doc_hash, doc_size) = Sim::from_document()?;
    info!("scene scenes/net-demo.json = {doc_hash:016x} ({doc_size} B)");

    let link = server.accept(t).context("accept")?;
    let mut session = Session::host(link, 1, t).context("host handshake")?;
    info!("host: session {} established", session.session_id);

    let mut snapshot_bytes = 0usize;
    for step in 1..=steps {
        host_sim.step(step);
        if step == replicate_at {
            snapshot_bytes = send_world_snapshot(&mut session, u64::from(step), &host_sim.world, t)
                .context("snapshot send")?;
            info!("host: world replicated at step {step} ({snapshot_bytes} B payload)");
        }
    }

    let host_final = save_json(&capture_world(&host_sim.world)).context("host capture")?;
    let client_out = client
        .join()
        .map_err(|_| anyhow::anyhow!("client thread panicked"))??;
    let (ticks_line, client_final) = client_out.split_once('\n').context("client output shape")?;
    let client_ticks: f64 = ticks_line
        .strip_prefix("TICKS:")
        .context("ticks prefix")?
        .parse()
        .context("ticks value")?;

    // Where did the agent end up? (Movement proof, from the document.)
    let save: SaveWorld = serde_json::from_str(&host_final).context("host parse")?;
    let agent = save
        .entities
        .iter()
        .find(|e| e.name.as_deref() == Some("patrol_agent"))
        .expect("agent in save");
    let agent_x = agent.transform.as_ref().expect("agent transform").position[0];

    let identical = host_final == client_final;
    let evidence = Evidence {
        protocol_version: SESSION_PROTOCOL_VERSION,
        session_id: 1,
        scene_key: "scenes/net-demo.json".into(),
        scene_hash: doc_hash,
        scene_bytes: doc_size,
        steps,
        replicate_at,
        snapshot_bytes,
        host_final,
        client_final: client_final.to_string(),
        identical,
        client_ticks,
        agent_x,
    };

    // Gates (honest: fail loudly, no "success" without evidence).
    anyhow::ensure!(evidence.identical, "host and client final worlds differ");
    anyhow::ensure!(
        (evidence.client_ticks - f64::from(steps)).abs() < 0.5,
        "client tick counter {} != {steps} — the script state did not survive the wire",
        evidence.client_ticks
    );
    anyhow::ensure!(
        agent_x > 1.5,
        "agent did not patrol (agent_x {agent_x}, started at 1.0)"
    );
    anyhow::ensure!(snapshot_bytes > 0, "nothing was replicated");

    let json = serde_json::to_string_pretty(&evidence).context("evidence serialize")?;
    std::fs::write(&out, &json).with_context(|| format!("write {}", out.display()))?;
    info!(
        "net_hello: steps {steps}, replicated at {replicate_at}, snapshot {snapshot_bytes} B, \
         host/client captures IDENTICAL, client ticks {}, evidence -> {} ({} B)",
        evidence.client_ticks,
        out.display(),
        json.len()
    );
    Ok(())
}
