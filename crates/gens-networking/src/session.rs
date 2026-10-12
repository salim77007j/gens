//! Sessions: a versioned, hash-checked message protocol on top of the
//! transport (Phase 6).
//!
//! A [`Session`] wraps one established [`QuicLink`] and adds:
//!
//! - a **handshake** — the client says `HELLO` (magic + protocol
//!   version), the host answers `WELCOME` (magic + version + session
//!   id) or `REJECT` (magic + reason). Talking to the wrong peer or a
//!   wrong protocol version fails loudly, not weirdly;
//! - a **message envelope** — every application message carries its
//!   simulation tick, a strict message kind and an end-to-end content
//!   hash. The transport already hash-checks each frame; the envelope
//!   hash stays independent of the transport so a future transport
//!   swap cannot silently weaken integrity (belt and suspenders,
//!   deliberate).
//!
//! Wire formats (little-endian throughout):
//!
//! ```text
//! HELLO   "GENH" | version: u16
//! WELCOME "GENW" | version: u16 | session_id: u64
//! REJECT  "GENR" | reason_len: u16 | reason bytes
//! ENVELOPE tick: u64 | kind: u8 | payload_len: u32 | payload_hash: u64 | payload
//! ```
//!
//! Determinism contract: envelopes are pure functions of their
//! content ([`decode_envelope`] is a pure, unit-tested function);
//! nothing here reads the wall clock for content — ticks are assigned
//! by the caller from the simulation.

use std::time::Duration;

use gens_assets::fnv1a64;

use crate::transport::{QuicLink, TransportError};

/// The only session protocol version this engine speaks.
pub const SESSION_PROTOCOL_VERSION: u16 = 1;

/// Largest envelope payload: the transport frame minus the envelope
/// header (tick + kind + payload_len + payload_hash).
pub const MAX_SESSION_PAYLOAD: usize = crate::transport::MAX_FRAME_BYTES as usize - 21;

/// Strict message vocabulary. Unknown kinds are rejected loudly —
/// a silent `Custom(u8)` escape hatch would let two disagreeing
/// builds "work" while corrupting each other's semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsgKind {
    /// A world snapshot (replication; payload = a versioned save
    /// document's canonical JSON — the mapping lives in gens-runtime).
    Snapshot,
    /// Player/controller input for a tick.
    Input,
    /// Polite goodbye; the receiver may close its side.
    Bye,
}

impl MsgKind {
    fn from_byte(b: u8) -> Option<MsgKind> {
        match b {
            1 => Some(MsgKind::Snapshot),
            2 => Some(MsgKind::Input),
            3 => Some(MsgKind::Bye),
            _ => None,
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            MsgKind::Snapshot => 1,
            MsgKind::Input => 2,
            MsgKind::Bye => 3,
        }
    }

    /// Stable name for logs and errors.
    pub fn as_str(self) -> &'static str {
        match self {
            MsgKind::Snapshot => "snapshot",
            MsgKind::Input => "input",
            MsgKind::Bye => "bye",
        }
    }
}

/// One received message: kind, tick, payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    /// The simulation tick the sender tagged the message with.
    pub tick: u64,
    /// The message kind.
    pub kind: MsgKind,
    /// The payload bytes.
    pub payload: Vec<u8>,
}

/// Session protocol failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The peer answered the handshake with a REJECT.
    #[error("session rejected by peer: {reason}")]
    Rejected {
        /// The peer's reason string.
        reason: String,
    },
    /// The handshake bytes were not what the protocol demands.
    #[error("session handshake failed: {0}")]
    Handshake(String),
    /// The peer speaks another protocol version.
    #[error("protocol version mismatch: peer {peer}, ours {SESSION_PROTOCOL_VERSION}")]
    VersionMismatch {
        /// The version the peer announced.
        peer: u16,
    },
    /// An envelope was malformed (bad length, unknown kind, ...).
    #[error("malformed message: {0}")]
    Malformed(String),
    /// The payload's content hash did not match.
    #[error("payload corrupt: recorded hash {recorded:#018x}, computed {computed:#018x}")]
    PayloadHashMismatch {
        /// Hash carried in the envelope.
        recorded: u64,
        /// Hash recomputed over the payload.
        computed: u64,
    },
    /// The payload exceeded [`MAX_SESSION_PAYLOAD`].
    #[error("payload too large: {len} bytes (max {max})")]
    PayloadTooLarge {
        /// Declared payload length.
        len: usize,
        /// [`MAX_SESSION_PAYLOAD`].
        max: usize,
    },
    /// The underlying transport failed.
    #[error("transport failure: {0}")]
    Transport(#[from] TransportError),
}

/// Convenience alias for session results.
pub type SessionResult<T> = Result<T, SessionError>;

/// A session message envelope (pure codec — unit-testable without a
/// socket).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Sender's simulation tick.
    pub tick: u64,
    /// Message kind.
    pub kind: MsgKind,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Encodes an envelope: tick, kind, payload length, payload hash, payload.
pub fn encode_envelope(env: &Envelope) -> SessionResult<Vec<u8>> {
    if env.payload.len() > MAX_SESSION_PAYLOAD {
        return Err(SessionError::PayloadTooLarge {
            len: env.payload.len(),
            max: MAX_SESSION_PAYLOAD,
        });
    }
    let mut buf = Vec::with_capacity(21 + env.payload.len());
    buf.extend_from_slice(&env.tick.to_le_bytes());
    buf.push(env.kind.to_byte());
    buf.extend_from_slice(&(env.payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&fnv1a64(&env.payload).to_le_bytes());
    buf.extend_from_slice(&env.payload);
    Ok(buf)
}

/// Decodes and fully validates an envelope (pure).
pub fn decode_envelope(wire: &[u8]) -> SessionResult<Envelope> {
    if wire.len() < 21 {
        return Err(SessionError::Malformed(format!(
            "envelope too short: {} bytes (min 21)",
            wire.len()
        )));
    }
    let tick = u64::from_le_bytes(wire[0..8].try_into().expect("8 bytes"));
    let kind_byte = wire[8];
    let kind = MsgKind::from_byte(kind_byte)
        .ok_or_else(|| SessionError::Malformed(format!("unknown message kind {kind_byte}")))?;
    let payload_len = u32::from_le_bytes(wire[9..13].try_into().expect("4 bytes")) as usize;
    if payload_len > MAX_SESSION_PAYLOAD {
        return Err(SessionError::PayloadTooLarge {
            len: payload_len,
            max: MAX_SESSION_PAYLOAD,
        });
    }
    let expected = 21 + payload_len;
    if wire.len() != expected {
        return Err(SessionError::Malformed(format!(
            "envelope length mismatch: header says {expected} bytes, got {}",
            wire.len()
        )));
    }
    let recorded = u64::from_le_bytes(wire[13..21].try_into().expect("8 bytes"));
    let payload = wire[21..].to_vec();
    let computed = fnv1a64(&payload);
    if recorded != computed {
        return Err(SessionError::PayloadHashMismatch { recorded, computed });
    }
    Ok(Envelope {
        tick,
        kind,
        payload,
    })
}

fn hello(version: u16) -> Vec<u8> {
    let mut buf = Vec::with_capacity(6);
    buf.extend_from_slice(b"GENH");
    buf.extend_from_slice(&version.to_le_bytes());
    buf
}

fn welcome(version: u16, session_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(14);
    buf.extend_from_slice(b"GENW");
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&session_id.to_le_bytes());
    buf
}

fn reject(reason: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(6 + reason.len());
    buf.extend_from_slice(b"GENR");
    buf.extend_from_slice(&(reason.len() as u16).to_le_bytes());
    buf.extend_from_slice(reason.as_bytes());
    buf
}

fn parse_handshake(wire: &[u8], expect_magic: &[u8; 4]) -> SessionResult<Option<(u16, Vec<u8>)>> {
    if wire.len() < 4 {
        return Err(SessionError::Handshake(format!(
            "handshake message too short: {} bytes",
            wire.len()
        )));
    }
    let magic = &wire[0..4];
    if magic != expect_magic {
        // A REJECT is a legal answer to a HELLO on the client side; the
        // caller decides what it means for its role.
        if magic == b"GENR" {
            if wire.len() < 6 {
                return Err(SessionError::Handshake("REJECT too short".into()));
            }
            let reason_len = u16::from_le_bytes(wire[4..6].try_into().expect("2 bytes")) as usize;
            let reason =
                String::from_utf8_lossy(&wire[6..6 + reason_len.min(wire.len() - 6)]).into_owned();
            return Ok(Some((0, reason.into_bytes())));
        }
        return Err(SessionError::Handshake(format!(
            "unexpected handshake magic {magic:?} (expected {expect_magic:?})"
        )));
    }
    if wire.len() < 6 {
        return Err(SessionError::Handshake("handshake missing version".into()));
    }
    let _version = u16::from_le_bytes(wire[4..6].try_into().expect("2 bytes"));
    Ok(None)
}

/// A versioned, hash-checked message session over one link.
pub struct Session {
    link: QuicLink,
    /// Assigned by the host at handshake time.
    pub session_id: u64,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Host side of the handshake: expect HELLO, answer WELCOME.
    ///
    /// `session_id` is assigned by the caller (the host's session
    /// manager) — this layer never invents identity.
    pub fn host(mut link: QuicLink, session_id: u64, timeout: Duration) -> SessionResult<Session> {
        let hello = link.recv_frame(timeout)?;
        if let Some((_, reason)) = parse_handshake(&hello, b"GENH")? {
            return Err(SessionError::Handshake(format!(
                "expected HELLO, got REJECT: {}",
                String::from_utf8_lossy(&reason)
            )));
        }
        let version = u16::from_le_bytes(hello[4..6].try_into().expect("2 bytes"));
        if version != SESSION_PROTOCOL_VERSION {
            link.send_frame(&reject("protocol version mismatch"), timeout)?;
            return Err(SessionError::VersionMismatch { peer: version });
        }
        link.send_frame(&welcome(SESSION_PROTOCOL_VERSION, session_id), timeout)?;
        Ok(Session { link, session_id })
    }

    /// Client side of the handshake: send HELLO, expect WELCOME.
    pub fn client(mut link: QuicLink, timeout: Duration) -> SessionResult<Session> {
        link.send_frame(&hello(SESSION_PROTOCOL_VERSION), timeout)?;
        let answer = link.recv_frame(timeout)?;
        if let Some((_, reason)) = parse_handshake(&answer, b"GENW")? {
            return Err(SessionError::Rejected {
                reason: String::from_utf8_lossy(&reason).into_owned(),
            });
        }
        let version = u16::from_le_bytes(answer[4..6].try_into().expect("2 bytes"));
        if version != SESSION_PROTOCOL_VERSION {
            return Err(SessionError::VersionMismatch { peer: version });
        }
        let session_id = u64::from_le_bytes(answer[6..14].try_into().expect("8 bytes"));
        Ok(Session { link, session_id })
    }

    /// Sends one envelope (blocking, bounded by `timeout`).
    pub fn send(
        &mut self,
        kind: MsgKind,
        tick: u64,
        payload: &[u8],
        timeout: Duration,
    ) -> SessionResult<()> {
        let wire = encode_envelope(&Envelope {
            tick,
            kind,
            payload: payload.to_vec(),
        })?;
        self.link.send_frame(&wire, timeout)?;
        Ok(())
    }

    /// Receives one envelope (blocking, bounded by `timeout`).
    pub fn recv(&mut self, timeout: Duration) -> SessionResult<IncomingMessage> {
        let wire = self.link.recv_frame(timeout)?;
        let env = decode_envelope(&wire)?;
        Ok(IncomingMessage {
            tick: env.tick,
            kind: env.kind,
            payload: env.payload,
        })
    }

    /// Closes the session with an application close code + reason.
    pub fn close(&mut self, code: u32, reason: &str) {
        self.link.close(code, reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LOOPBACK_SERVER_NAME;
    use crate::cert::LoopbackCert;
    use crate::transport::QuicEndpoint;
    use std::net::SocketAddr;

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn envelope_layout_is_exact() {
        let env = Envelope {
            tick: 7,
            kind: MsgKind::Snapshot,
            payload: b"ab".to_vec(),
        };
        let wire = encode_envelope(&env).expect("encode");
        assert_eq!(&wire[0..8], &7u64.to_le_bytes());
        assert_eq!(wire[8], 1, "snapshot kind byte");
        assert_eq!(&wire[9..13], &2u32.to_le_bytes());
        assert_eq!(&wire[13..21], &gens_assets::fnv1a64(b"ab").to_le_bytes());
        assert_eq!(&wire[21..], b"ab");
        assert_eq!(wire.len(), 23);
    }

    #[test]
    fn envelope_round_trips_all_kinds() {
        for (kind, payload) in [
            (MsgKind::Snapshot, vec![1u8, 2, 3]),
            (MsgKind::Input, vec![]),
            (MsgKind::Bye, b"goodbye".to_vec()),
        ] {
            let wire = encode_envelope(&Envelope {
                tick: 42,
                kind,
                payload: payload.clone(),
            })
            .expect("encode");
            let back = decode_envelope(&wire).expect("decode");
            assert_eq!(back.tick, 42);
            assert_eq!(back.kind, kind);
            assert_eq!(back.payload, payload);
        }
    }

    #[test]
    fn envelope_rejects_unknown_kind_and_bad_lengths() {
        let mut wire = encode_envelope(&Envelope {
            tick: 1,
            kind: MsgKind::Input,
            payload: vec![9],
        })
        .expect("encode");
        wire[8] = 200; // unknown kind
        assert!(matches!(
            decode_envelope(&wire),
            Err(SessionError::Malformed(_))
        ));
        // Header claims more payload than present.
        let mut wire = encode_envelope(&Envelope {
            tick: 1,
            kind: MsgKind::Input,
            payload: vec![9],
        })
        .expect("encode");
        wire[9..13].copy_from_slice(&5u32.to_le_bytes());
        assert!(matches!(
            decode_envelope(&wire),
            Err(SessionError::Malformed(_))
        ));
        // Truncated envelope.
        let wire = encode_envelope(&Envelope {
            tick: 1,
            kind: MsgKind::Input,
            payload: vec![9],
        })
        .expect("encode");
        assert!(matches!(
            decode_envelope(&wire[..10]),
            Err(SessionError::Malformed(_))
        ));
        assert!(matches!(
            decode_envelope(&[]),
            Err(SessionError::Malformed(_))
        ));
    }

    #[test]
    fn envelope_payload_hash_mismatch_is_loud() {
        let mut wire = encode_envelope(&Envelope {
            tick: 1,
            kind: MsgKind::Input,
            payload: b"trust me".to_vec(),
        })
        .expect("encode");
        // Flip a payload byte after the header was written.
        let last = wire.len() - 1;
        wire[last] ^= 0xFF;
        let err = decode_envelope(&wire).expect_err("corrupt payload rejected");
        assert!(
            matches!(err, SessionError::PayloadHashMismatch { .. }),
            "{err}"
        );
    }

    #[test]
    fn envelope_payload_size_limits() {
        let big = vec![0u8; MAX_SESSION_PAYLOAD + 1];
        let err = encode_envelope(&Envelope {
            tick: 1,
            kind: MsgKind::Input,
            payload: big,
        })
        .expect_err("oversize rejected");
        assert!(matches!(err, SessionError::PayloadTooLarge { .. }), "{err}");
        // Exactly at the limit is fine (encode only; decode needs the
        // transport frame to carry it, which this pure test skips).
        let at_limit = vec![0u8; MAX_SESSION_PAYLOAD];
        assert!(
            encode_envelope(&Envelope {
                tick: 1,
                kind: MsgKind::Input,
                payload: at_limit,
            })
            .is_ok()
        );
    }

    #[test]
    fn session_handshake_and_messages_over_loopback() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let link = server.accept(T).expect("accept");
                let mut session = Session::host(link, 1, T).expect("host handshake");
                assert_eq!(session.session_id, 1);
                // Receive two messages in FIFO order.
                let m1 = session.recv(T).expect("recv 1");
                assert_eq!((m1.tick, m1.kind), (10, MsgKind::Snapshot));
                assert_eq!(m1.payload, b"world-one");
                let m2 = session.recv(T).expect("recv 2");
                assert_eq!((m2.tick, m2.kind), (11, MsgKind::Input));
                assert_eq!(m2.payload, b"");
                // Reply.
                session
                    .send(MsgKind::Snapshot, 12, b"ack-world", T)
                    .expect("send ack");
                let bye = session.recv(T).expect("recv bye");
                assert_eq!(bye.kind, MsgKind::Bye);
                done_tx.send(()).expect("report done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            let mut session = Session::client(link, T).expect("client handshake");
            assert_eq!(session.session_id, 1, "host-assigned id reaches client");
            session
                .send(MsgKind::Snapshot, 10, b"world-one", T)
                .expect("send 1");
            session.send(MsgKind::Input, 11, b"", T).expect("send 2");
            let ack = session.recv(T).expect("recv ack");
            assert_eq!(ack.payload, b"ack-world");
            session.send(MsgKind::Bye, 13, b"", T).expect("bye");
            done_rx.recv().expect("host finished");
        });
    }

    #[test]
    fn session_rejects_version_mismatch() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let link = server.accept(T).expect("accept");
                let err = Session::host(link, 1, T).expect_err("version mismatch");
                assert!(
                    matches!(err, SessionError::VersionMismatch { peer: 99 }),
                    "{err}"
                );
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            // Speak a future version by hand.
            let mut hello = Vec::new();
            hello.extend_from_slice(b"GENH");
            hello.extend_from_slice(&99u16.to_le_bytes());
            let mut session_link = link;
            session_link.send_frame(&hello, T).expect("hello");
            let answer = session_link.recv_frame(T).expect("reject");
            assert!(answer.starts_with(b"GENR"), "host sent REJECT");
        });
    }
}
