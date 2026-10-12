//! Sessions, replication and transport (Phase 6).
//!
//! The engine's networking layer is **loopback-first**: every primitive
//! is developed and CI-verified as an in-process host/client pair over
//! real QUIC on 127.0.0.1, with self-signed certificates generated at
//! bind time ([`cert`]) and a synchronous facade that owns its tokio
//! runtime ([`transport`]) — the engine itself never sees async.
//!
//! Pillars (per the README roadmap):
//! - [`transport`] (P6-3): QUIC endpoints + FNV-verified framed
//!   messages — the integrity discipline the asset registry and the
//!   save format already use, now on the wire;
//! - sessions + replication (P6-4): deterministic session state and
//!   snapshot transfer built on the save format's mirror types;
//! - the integration proof `net_hello` (P6-5) composes all of it with
//!   the data-document world bootstrap ([`gens_assets::data`]).
//!
//! Determinism contract: nothing here reads the wall clock for
//! *content* — timeouts bound operations, they never enter messages.
//! Cross-platform float bit-identity is NOT assumed (R-013): what
//! crosses the wire is hashes and canonical documents, both pure
//! functions of content.

pub mod cert;
pub mod session;
pub mod transport;

pub use cert::{LOOPBACK_SERVER_NAME, LoopbackCert};
pub use session::{
    Envelope, IncomingMessage, MsgKind, SESSION_PROTOCOL_VERSION, Session, SessionError,
    SessionResult, decode_envelope, encode_envelope,
};
pub use transport::{MAX_FRAME_BYTES, QuicEndpoint, QuicLink, TransportError, TransportResult};
