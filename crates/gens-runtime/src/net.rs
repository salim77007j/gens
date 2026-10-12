//! Networking bridge: world snapshots over a session (Phase 6).
//!
//! The runtime half of the replication story: a [`gens_ecs`] world's
//! saveable state ([`crate::save::SaveWorld`], the Phase 5 mirror
//! document) travels over a [`gens_networking::Session`] as a
//! `Snapshot` message whose payload is the save document's canonical
//! JSON. Identity and integrity ride three independent layers:
//!
//! 1. the transport frame's FNV hash (corruption on the wire),
//! 2. the session envelope's payload hash (end-to-end, transport-
//!    independent),
//! 3. the save document's own hash when it is later written to disk.
//!
//! Determinism contract: `capture_world` output is byte-stable
//! (canonical entity order, BTreeMap bags), so the same world state
//! always produces the same payload — a snapshot hash is therefore a
//! world-state fingerprint, usable in evidence ledgers.

use gens_ecs::ecs::world::World;
use gens_networking::{MsgKind, Session, SessionError};

use crate::save::{SaveWorld, capture_world, save_json};

/// Errors raised by the runtime networking bridge.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NetError {
    /// The session failed.
    #[error("session failure: {0}")]
    Session(#[from] SessionError),
    /// A message arrived that was not the expected kind.
    #[error("unexpected message kind: expected {expected}, got {got}")]
    UnexpectedKind {
        /// The kind the receiver wanted.
        expected: &'static str,
        /// The kind that arrived.
        got: &'static str,
    },
    /// The snapshot payload was not a valid save document.
    #[error("snapshot payload invalid: {0}")]
    BadSnapshot(String),
}

/// Convenience alias for net-bridge results.
pub type NetResult<T> = Result<T, NetError>;

/// Captures `world` and sends it as a `Snapshot` message tagged with
/// `tick`. Returns the payload size in bytes.
pub fn send_world_snapshot(
    session: &mut Session,
    tick: u64,
    world: &World,
    timeout: std::time::Duration,
) -> NetResult<usize> {
    let save = capture_world(world);
    let json = save_json(&save).map_err(|e| NetError::BadSnapshot(e.to_string()))?;
    let payload = json.into_bytes();
    let len = payload.len();
    session.send(MsgKind::Snapshot, tick, &payload, timeout)?;
    Ok(len)
}

/// Receives one message and, if it is a `Snapshot`, parses the save
/// document. Any other kind is a loud [`NetError::UnexpectedKind`].
pub fn recv_world_snapshot(
    session: &mut Session,
    timeout: std::time::Duration,
) -> NetResult<(u64, SaveWorld)> {
    let msg = session.recv(timeout)?;
    if msg.kind != MsgKind::Snapshot {
        return Err(NetError::UnexpectedKind {
            expected: "snapshot",
            got: msg.kind.as_str(),
        });
    }
    let save: SaveWorld = serde_json::from_slice(&msg.payload)
        .map_err(|e| NetError::BadSnapshot(format!("save document parse failed: {e}")))?;
    Ok((msg.tick, save))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_networking::{LOOPBACK_SERVER_NAME, LoopbackCert, QuicEndpoint};
    use std::net::SocketAddr;
    use std::time::Duration;

    const T: Duration = Duration::from_secs(10);

    /// One scripted agent + one static beacon, built through the P6-2
    /// data bridge — the same scene net_hello replicates.
    fn demo_world() -> World {
        const DOC: &str = r#"{
            "format_version": 1,
            "name": "net-demo",
            "entities": [
                {
                    "name": "agent",
                    "translation": [1.0, 0.0, 1.0],
                    "script": {
                        "script": "patrol",
                        "entry": "update",
                        "vars": {"wp": {"kind": "number", "value": 0.0}}
                    }
                },
                {"name": "beacon", "translation": [9.0, 2.0, 7.0], "tag": "landmark"}
            ]
        }"#;
        let doc = gens_assets::parse_data_document("scenes/net-demo.json", DOC.as_bytes())
            .expect("valid doc");
        let mut world = World::new();
        let mut host = gens_scripting::ScriptHost::new();
        host.register("patrol", "fn update() {}").expect("register");
        world.insert_resource(host);
        crate::data::apply_data_document(&mut world, &doc).expect("applies");
        world
    }

    #[test]
    fn snapshot_round_trips_over_loopback_session() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let link = server.accept(T).expect("accept");
                let mut session = Session::host(link, 1, T).expect("host");
                let (tick, save) = recv_world_snapshot(&mut session, T).expect("recv");
                assert_eq!(tick, 77);
                assert_eq!(save.format_version, crate::save::FORMAT_VERSION);
                // The agent and the beacon both survived the wire.
                let mut names: Vec<String> = save
                    .entities
                    .iter()
                    .filter_map(|e| e.name.clone())
                    .collect();
                names.sort();
                assert_eq!(names, vec!["agent".to_string(), "beacon".to_string()]);
                done_tx.send(()).expect("done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            let mut session = Session::client(link, T).expect("client");
            let world = demo_world();
            let sent = send_world_snapshot(&mut session, 77, &world, T).expect("send");
            assert!(sent > 100, "snapshot payload carries the scene: {sent} B");
            done_rx.recv().expect("host done");
        });
    }

    #[test]
    fn unexpected_kind_is_loud() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let link = server.accept(T).expect("accept");
                let mut session = Session::host(link, 1, T).expect("host");
                let err = recv_world_snapshot(&mut session, T).expect_err("wrong kind");
                assert!(
                    matches!(
                        err,
                        NetError::UnexpectedKind {
                            expected: "snapshot",
                            got: "input"
                        }
                    ),
                    "{err}"
                );
                done_tx.send(()).expect("done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            let mut session = Session::client(link, T).expect("client");
            session
                .send(MsgKind::Input, 1, b"not a snapshot", T)
                .expect("send input");
            done_rx.recv().expect("host done");
        });
    }
}
