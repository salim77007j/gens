//! QUIC loopback transport behind a synchronous facade (Phase 6).
//!
//! The engine is not async. The transport **owns its runtime**: each
//! [`QuicEndpoint`] runs a dedicated I/O thread whose tokio
//! `current_thread` runtime is blocked on a park loop for the
//! endpoint's whole life, so quinn's endpoint/connection drivers are
//! polled continuously.
//!
//! WHY THIS SHAPE (empirically established during P6-3, kept as a
//! documented constraint): quinn transmits packets only while its
//! runtime is being `block_on`-driven. A facade that starts and stops
//! `block_on` per call freezes outbound frames the moment its main
//! future completes — a handshake can complete against a live peer
//! while the peer's *reply* sits forever in an unpolled send queue.
//! The permanently-driven I/O thread removes that entire failure
//! class; the facade's synchronous calls then only enqueue work and
//! await a bounded reply.
//!
//! Wire discipline (one bidirectional stream per link, both directions):
//!
//! ```text
//! [u32 LE payload length N][u64 LE FNV-1a64(payload)][N payload bytes]
//! ```
//!
//! Link establishment ends with an **announce frame**: the client
//! sends one empty frame immediately after opening the stream, and
//! the server consumes it during `accept`. This is required because
//! QUIC announces a stream on the wire only when its first byte is
//! written — a client that opens a stream but stays silent would
//! otherwise leave the server's `accept_bi` waiting forever (found
//! empirically in P6-3: three tests deadlocked exactly there).
//!
//! Every frame is hash-verified on receipt — the same FNV-1a 64 the
//! asset registry and the save format use — so silent corruption is
//! impossible and tampering is loud. Frames above [`MAX_FRAME_BYTES`]
//! are rejected before allocation.
//!
//! Threading model: one caller thread per endpoint at a time. Each
//! link's send/recv commands are handled FIFO by a per-link manager
//! task, so per-link ordering is preserved and the quinn streams are
//! never accessed concurrently.

use std::net::SocketAddr;
use std::time::Duration;

use crate::cert::LoopbackCert;
use gens_assets::fnv1a64;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// Largest accepted frame payload: 16 MiB. Anything bigger is treated
/// as corruption (a 4-byte length prefix pointing at gigabytes is how
/// a hostile or broken peer would OOM us).
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Extra time the *caller* waits past an operation's own timeout for
/// the I/O thread's reply (the I/O thread enforces the operation
/// timeout itself; the margin only covers channel latency).
const REPLY_MARGIN: Duration = Duration::from_secs(2);

/// Transport errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// Binding the endpoint socket failed.
    #[error("endpoint bind failed on {addr}: {source}")]
    Bind {
        /// Address we tried to bind.
        addr: SocketAddr,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// Certificate generation failed.
    #[error("loopback certificate generation failed: {0}")]
    Cert(#[from] rcgen::Error),
    /// TLS configuration failed (server cert or client roots).
    #[error("TLS configuration failed: {0}")]
    CryptoConfig(String),
    /// The client could not reach or handshake with the server.
    #[error("connect to {addr} failed: {source}")]
    Connect {
        /// Address we tried to reach.
        addr: SocketAddr,
        /// quinn's diagnostics.
        #[source]
        source: quinn::ConnectError,
    },
    /// The connection was lost (closed by peer, idle timeout, ...).
    #[error("connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),
    /// Reading the stream failed.
    #[error("stream read failed: {0}")]
    Read(#[from] quinn::ReadExactError),
    /// Writing the stream failed.
    #[error("stream write failed: {0}")]
    Write(#[from] quinn::WriteError),
    /// The operation exceeded its timeout.
    #[error("operation timed out after {timeout_ms} ms")]
    Timeout {
        /// The timeout that fired.
        timeout_ms: u64,
    },
    /// A received frame's hash did not match its payload.
    #[error("frame corrupt: recorded hash {recorded:#018x}, computed {computed:#018x}")]
    CorruptFrame {
        /// Hash carried in the frame.
        recorded: u64,
        /// Hash recomputed over the payload.
        computed: u64,
    },
    /// A frame declared a payload larger than [`MAX_FRAME_BYTES`].
    #[error("frame too large: {len} bytes (max {max})")]
    FrameTooLarge {
        /// Declared length.
        len: u32,
        /// [`MAX_FRAME_BYTES`].
        max: u32,
    },
    /// The endpoint or link was closed locally / by the peer.
    #[error("closed: {0}")]
    Closed(String),
    /// The I/O thread stopped answering (it is a process-lifetime
    /// daemon; this means the process is tearing down).
    #[error("I/O thread is unreachable")]
    IoThreadGone,
}

/// Convenience alias for transport results.
pub type TransportResult<T> = Result<T, TransportError>;

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current_thread runtime builds")
}

fn local_ephemeral() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// Commands the public facade sends to the endpoint's manager task.
enum EndpointCmd {
    CloseEndpoint {
        reason: String,
    },
    /// Last command: drain queued work for `grace`, then end the
    /// manager so the I/O thread can exit cleanly (teardown frames —
    /// CONNECTION_CLOSE, stream resets — get transmitted instead of
    /// being lost to a runtime drop).
    Shutdown {
        grace: Duration,
    },
    Connect {
        addr: SocketAddr,
        server_name: String,
        timeout: Duration,
        reply: std::sync::mpsc::Sender<TransportResult<LinkMailbox>>,
    },
    Accept {
        timeout: Duration,
        reply: std::sync::mpsc::Sender<TransportResult<LinkMailbox>>,
    },
}

/// A link's private mailbox: the per-link manager task serves these
/// FIFO, so frame ordering is preserved end to end.
enum LinkCmd {
    Send {
        payload: Vec<u8>,
        timeout: Duration,
        reply: std::sync::mpsc::Sender<TransportResult<()>>,
    },
    Recv {
        timeout: Duration,
        reply: std::sync::mpsc::Sender<TransportResult<Vec<u8>>>,
    },
    Close {
        code: u32,
        reason: String,
    },
}

/// The per-link state owned by its manager task on the I/O runtime.
struct LinkInner {
    connection: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

type LinkMailbox = mpsc::Sender<LinkCmd>;

/// Serves one link's mailbox FIFO until the mailbox closes (link
/// dropped) or the connection dies.
async fn link_manager(mut mailbox: mpsc::Receiver<LinkCmd>, mut inner: LinkInner) {
    while let Some(cmd) = mailbox.recv().await {
        match cmd {
            LinkCmd::Send {
                payload,
                timeout,
                reply,
            } => {
                let result = if payload.len() > MAX_FRAME_BYTES as usize {
                    Err(TransportError::FrameTooLarge {
                        len: payload.len() as u32,
                        max: MAX_FRAME_BYTES,
                    })
                } else {
                    let frame = frame_of(&payload);
                    match tokio::time::timeout(timeout, inner.send.write_all(&frame)).await {
                        Ok(Ok(())) => {
                            // The I/O runtime is permanently driven, so
                            // the queued bytes are transmitted by the
                            // connection driver without further help.
                            Ok(())
                        }
                        Ok(Err(e)) => Err(TransportError::Write(e)),
                        Err(_) => Err(TransportError::Timeout {
                            timeout_ms: timeout.as_millis() as u64,
                        }),
                    }
                };
                let _ = reply.send(result);
            }
            LinkCmd::Recv { timeout, reply } => {
                let result = tokio::time::timeout(timeout, async {
                    let mut len_buf = [0u8; 4];
                    inner.recv.read_exact(&mut len_buf).await?;
                    let len = u32::from_le_bytes(len_buf);
                    // The allocation guard fires BEFORE the payload read.
                    if len > MAX_FRAME_BYTES {
                        return Err(TransportError::FrameTooLarge {
                            len,
                            max: MAX_FRAME_BYTES,
                        });
                    }
                    let mut hash_buf = [0u8; 8];
                    inner.recv.read_exact(&mut hash_buf).await?;
                    let recorded = u64::from_le_bytes(hash_buf);
                    let mut payload = vec![0u8; len as usize];
                    inner.recv.read_exact(&mut payload).await?;
                    verify_frame(len, recorded, &payload)
                })
                .await
                .unwrap_or_else(|_| {
                    Err(TransportError::Timeout {
                        timeout_ms: timeout.as_millis() as u64,
                    })
                });
                let _ = reply.send(result);
            }
            LinkCmd::Close { code, reason } => {
                inner.connection.close(code.into(), reason.as_bytes());
                // Bounded drain: the runtime is driven until the
                // endpoint's Shutdown grace elapses, so the
                // CONNECTION_CLOSE is transmitted, not queued-then-lost.
                tokio::time::sleep(Duration::from_millis(30)).await;
                break;
            }
        }
    }
    // Mailbox closed (link dropped without an explicit close): bounded
    // drain so any teardown frames queued by the drops reach the wire.
    tokio::time::sleep(Duration::from_millis(30)).await;
}

/// Verifies a received frame's integrity (pure; unit-testable — the
/// facade deliberately does not expose raw streams, so hostile-peer
/// bytes are pinned at this function).
fn verify_frame(len: u32, recorded: u64, payload: &[u8]) -> TransportResult<Vec<u8>> {
    if len > MAX_FRAME_BYTES {
        return Err(TransportError::FrameTooLarge {
            len,
            max: MAX_FRAME_BYTES,
        });
    }
    if len as usize != payload.len() {
        return Err(TransportError::FrameTooLarge {
            len,
            max: MAX_FRAME_BYTES,
        });
    }
    let computed = fnv1a64(payload);
    if recorded != computed {
        return Err(TransportError::CorruptFrame { recorded, computed });
    }
    Ok(payload.to_vec())
}

/// Encodes a frame: length prefix + FNV hash + payload.
fn frame_of(payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&fnv1a64(payload).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// A bound QUIC endpoint (server or client role at bind time) with a
/// dedicated, permanently-driven I/O thread. Loopback-only by
/// construction: [`QuicEndpoint::server`] presents a self-signed
/// certificate and [`QuicEndpoint::client`] trusts exactly one
/// operator-provided certificate.
pub struct QuicEndpoint {
    cmd: mpsc::Sender<EndpointCmd>,
    local: SocketAddr,
    io_thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for QuicEndpoint {
    fn drop(&mut self) {
        // Graceful teardown: let the I/O thread flush teardown frames
        // (closes queued by dropped links, endpoint closes) before the
        // runtime goes away. Best effort — a dead I/O thread is ignored.
        let shutdown_ok = self
            .cmd
            .blocking_send(EndpointCmd::Shutdown {
                grace: Duration::from_millis(250),
            })
            .is_ok();
        if shutdown_ok {
            let _ = self.io_thread.take().map(std::thread::JoinHandle::join);
        }
    }
}

impl std::fmt::Debug for QuicEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicEndpoint")
            .field("local_addr", &self.local)
            .finish_non_exhaustive()
    }
}

impl QuicEndpoint {
    /// Binds a server endpoint on `bind_addr` (use port 0 for an
    /// ephemeral port) presenting `cert`, and starts its I/O thread.
    pub fn server(bind_addr: SocketAddr, cert: &LoopbackCert) -> TransportResult<QuicEndpoint> {
        let server_config = cert
            .server_config()
            .map_err(|e| TransportError::CryptoConfig(format!("server config: {e}")))?;
        let runtime = runtime();
        let endpoint = runtime
            .block_on(async { quinn::Endpoint::server(server_config, bind_addr) })
            .map_err(|source| TransportError::Bind {
                addr: bind_addr,
                source,
            })?;
        let local = endpoint
            .local_addr()
            .expect("endpoint local address after successful bind");
        Ok(Self::spawn_io(runtime, endpoint, local))
    }

    /// Binds a client endpoint (ephemeral local port) that trusts only
    /// `cert` — the operator hands us the server's certificate DER;
    /// there is no network trust of any kind — and starts its I/O thread.
    pub fn client(cert: &LoopbackCert) -> TransportResult<QuicEndpoint> {
        let client_config = cert.client_config().map_err(TransportError::CryptoConfig)?;
        let runtime = runtime();
        let mut endpoint = runtime
            .block_on(async { quinn::Endpoint::client(local_ephemeral()) })
            .map_err(|source| TransportError::Bind {
                addr: local_ephemeral(),
                source,
            })?;
        endpoint.set_default_client_config(client_config);
        let local = endpoint
            .local_addr()
            .expect("endpoint local address after successful bind");
        Ok(Self::spawn_io(runtime, endpoint, local))
    }

    /// Starts the I/O thread + manager task. quinn resolves its async
    /// runtime when the endpoint/socket is created, which happened on
    /// this thread inside `block_on` above.
    fn spawn_io(runtime: Runtime, endpoint: quinn::Endpoint, local: SocketAddr) -> QuicEndpoint {
        let (tx, rx) = mpsc::channel::<EndpointCmd>(32);
        let io_thread = std::thread::Builder::new()
            .name("gens-net-io".into())
            .spawn(move || {
                // The I/O runtime lives until the manager returns (a
                // Shutdown command or every endpoint handle dropped).
                runtime.block_on(endpoint_manager(endpoint, rx));
            })
            .expect("spawning the gens-net-io thread");
        QuicEndpoint {
            cmd: tx,
            local,
            io_thread: Some(io_thread),
        }
    }

    /// The bound local address (after an ephemeral bind, read this to
    /// learn the port).
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Connects to `addr` and opens the link's single bi-directional
    /// stream. `server_name` must match the certificate's SAN
    /// ([`LOOPBACK_SERVER_NAME`] for loopback certs). The timeout
    /// bounds the handshake AND the stream open.
    pub fn connect(
        &self,
        addr: SocketAddr,
        server_name: &str,
        timeout: Duration,
    ) -> TransportResult<QuicLink> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.cmd
            .blocking_send(EndpointCmd::Connect {
                addr,
                server_name: server_name.to_string(),
                timeout,
                reply: reply_tx,
            })
            .map_err(|_| TransportError::IoThreadGone)?;
        let mailbox = reply_rx
            .recv_timeout(timeout + REPLY_MARGIN)
            .map_err(|_| TransportError::Timeout {
                timeout_ms: (timeout + REPLY_MARGIN).as_millis() as u64,
            })??;
        Ok(QuicLink { mailbox })
    }

    /// Accepts one incoming link and opens its stream. The timeout
    /// bounds the wait for BOTH the handshake and the client's stream
    /// open (the client opens its stream immediately after connecting).
    pub fn accept(&self, timeout: Duration) -> TransportResult<QuicLink> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.cmd
            .blocking_send(EndpointCmd::Accept {
                timeout,
                reply: reply_tx,
            })
            .map_err(|_| TransportError::IoThreadGone)?;
        let mailbox = reply_rx
            .recv_timeout(timeout + REPLY_MARGIN)
            .map_err(|_| TransportError::Timeout {
                timeout_ms: (timeout + REPLY_MARGIN).as_millis() as u64,
            })??;
        Ok(QuicLink { mailbox })
    }

    /// Closes the endpoint, rejecting pending and future connections
    /// with an application close carrying `reason`. Dropping the
    /// endpoint also stops serving new commands (the I/O thread is a
    /// process-lifetime daemon; its runtime simply goes idle).
    pub fn close(&self, reason: &str) {
        let _ = self.cmd.blocking_send(EndpointCmd::CloseEndpoint {
            reason: reason.to_string(),
        });
    }
}

/// The endpoint's manager task: serves connect/accept commands
/// concurrently (a pending accept must not block a connect).
async fn endpoint_manager(endpoint: quinn::Endpoint, mut rx: mpsc::Receiver<EndpointCmd>) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            EndpointCmd::CloseEndpoint { reason } => {
                endpoint.close(0u32.into(), reason.as_bytes());
            }
            EndpointCmd::Shutdown { grace } => {
                tokio::time::sleep(grace).await;
                break;
            }
            EndpointCmd::Connect {
                addr,
                server_name,
                timeout,
                reply,
            } => {
                let endpoint = endpoint.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(timeout, async {
                        let connecting = endpoint
                            .connect(addr, &server_name)
                            .map_err(|source| TransportError::Connect { addr, source })?;
                        let connection = connecting.await?;
                        let (mut send, recv) = connection.open_bi().await?;
                        // Announce the stream on the wire NOW: QUIC only
                        // sends the STREAM frame with the first byte, and
                        // the server's accept consumes this empty frame.
                        send.write_all(&frame_of(&[])).await?;
                        let (tx, rxx) = mpsc::channel(32);
                        tokio::spawn(link_manager(
                            rxx,
                            LinkInner {
                                connection,
                                send,
                                recv,
                            },
                        ));
                        Ok::<LinkMailbox, TransportError>(tx)
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(TransportError::Timeout {
                            timeout_ms: timeout.as_millis() as u64,
                        })
                    });
                    let _ = reply.send(result);
                });
            }
            EndpointCmd::Accept { timeout, reply } => {
                let endpoint = endpoint.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(timeout, async {
                        let incoming = endpoint
                            .accept()
                            .await
                            .ok_or_else(|| TransportError::Closed("endpoint closed".into()))?;
                        let connection = incoming.await?;
                        let (send, mut recv) = connection.accept_bi().await?;
                        // Consume the client's announce frame (empty
                        // payload): proves the stream is live and leaves
                        // the link clean for the first real frame.
                        let mut len_buf = [0u8; 4];
                        recv.read_exact(&mut len_buf).await?;
                        let len = u32::from_le_bytes(len_buf);
                        let mut hash_buf = [0u8; 8];
                        recv.read_exact(&mut hash_buf).await?;
                        let recorded = u64::from_le_bytes(hash_buf);
                        let mut payload = vec![0u8; len as usize];
                        recv.read_exact(&mut payload).await?;
                        verify_frame(len, recorded, &payload)?;
                        if len != 0 {
                            return Err(TransportError::Closed(
                                "link announce was not an empty frame".into(),
                            ));
                        }
                        let (tx, rxx) = mpsc::channel(32);
                        tokio::spawn(link_manager(
                            rxx,
                            LinkInner {
                                connection,
                                send,
                                recv,
                            },
                        ));
                        Ok::<LinkMailbox, TransportError>(tx)
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(TransportError::Timeout {
                            timeout_ms: timeout.as_millis() as u64,
                        })
                    });
                    let _ = reply.send(result);
                });
            }
        }
    }
}

/// One established, framed link between two endpoints.
///
/// Both sides see the same API: `send_frame` writes, `recv_frame`
/// reads and hash-verifies. The single bi-directional stream is opened
/// by the client during [`QuicEndpoint::connect`] and accepted by the
/// server during [`QuicEndpoint::accept`].
pub struct QuicLink {
    mailbox: LinkMailbox,
}

impl std::fmt::Debug for QuicLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuicLink").finish_non_exhaustive()
    }
}

impl QuicLink {
    /// Sends one framed message (blocking, bounded by `timeout`).
    pub fn send_frame(&mut self, payload: &[u8], timeout: Duration) -> TransportResult<()> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.mailbox
            .blocking_send(LinkCmd::Send {
                payload: payload.to_vec(),
                timeout,
                reply: reply_tx,
            })
            .map_err(|_| TransportError::Closed("link manager gone".into()))?;
        reply_rx
            .recv_timeout(timeout + REPLY_MARGIN)
            .map_err(|_| TransportError::Timeout {
                timeout_ms: (timeout + REPLY_MARGIN).as_millis() as u64,
            })?
    }

    /// Receives one framed message (blocking, bounded by `timeout`);
    /// verifies the FNV hash before returning the payload.
    pub fn recv_frame(&mut self, timeout: Duration) -> TransportResult<Vec<u8>> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.mailbox
            .blocking_send(LinkCmd::Recv {
                timeout,
                reply: reply_tx,
            })
            .map_err(|_| TransportError::Closed("link manager gone".into()))?;
        reply_rx
            .recv_timeout(timeout + REPLY_MARGIN)
            .map_err(|_| TransportError::Timeout {
                timeout_ms: (timeout + REPLY_MARGIN).as_millis() as u64,
            })?
    }

    /// Closes the link with an application-level close code + reason.
    /// The peer's next operation fails with [`TransportError::Connection`].
    pub fn close(&mut self, code: u32, reason: &str) {
        let _ = self.mailbox.blocking_send(LinkCmd::Close {
            code,
            reason: reason.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::LOOPBACK_SERVER_NAME;

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn frame_layout_is_exact() {
        let payload = b"a";
        let frame = frame_of(payload);
        // 4-byte LE length == 1
        assert_eq!(&frame[0..4], &1u32.to_le_bytes());
        // 8-byte LE FNV-1a64 of b"a" (published test vector)
        assert_eq!(&frame[4..12], &0xaf63dc4c8601ec8cu64.to_le_bytes());
        // payload
        assert_eq!(&frame[12..], b"a");
        assert_eq!(frame.len(), 4 + 8 + 1);
        // Empty payload is a valid frame.
        let empty = frame_of(b"");
        assert_eq!(empty.len(), 12);
        assert_eq!(&empty[0..4], &0u32.to_le_bytes());
        assert_eq!(&empty[4..12], &0xcbf29ce484222325u64.to_le_bytes());
    }

    #[test]
    fn loopback_send_recv_both_directions() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let mut link = server.accept(T).expect("accept");
                let got = link.recv_frame(T).expect("server recv");
                assert_eq!(got, b"hello from client");
                link.send_frame(b"hello from server", T)
                    .expect("server send");
                let second = link.recv_frame(T).expect("server recv 2");
                assert_eq!(second, b"");
                done_tx.send(()).expect("report done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let mut link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            link.send_frame(b"hello from client", T)
                .expect("client send");
            let got = link.recv_frame(T).expect("client recv");
            assert_eq!(got, b"hello from server");
            link.send_frame(b"", T).expect("empty frame send");
            // Deterministic ending: the link must outlive the exchange.
            done_rx.recv().expect("server finished");
        });
    }

    #[test]
    fn one_mebibyte_payload_round_trips() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        const PAYLOAD_LEN: usize = 1024 * 1024;
        let payload: Vec<u8> = (0..PAYLOAD_LEN).map(|i| (i % 251) as u8).collect();
        let expected = fnv1a64(&payload);
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let mut link = server.accept(T).expect("accept");
                let got = link.recv_frame(T).expect("server recv");
                assert_eq!(got.len(), PAYLOAD_LEN);
                assert_eq!(fnv1a64(&got), expected);
                link.send_frame(b"ack", T).expect("server ack");
                done_tx.send(()).expect("report done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let mut link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            link.send_frame(&payload, T).expect("client send");
            let ack = link.recv_frame(T).expect("client waits for ack");
            assert_eq!(ack, b"ack");
            done_rx.recv().expect("server finished");
        });
    }

    #[test]
    fn corrupt_frame_is_detected_with_hashes() {
        // The facade deliberately does not expose raw streams (every
        // byte a caller sends is framed with a correct hash), so the
        // hostile-peer path is pinned at the pure verifier the link
        // manager uses.
        let payload = b"x";
        let err = verify_frame(1, 0xdead_beef, payload).expect_err("corrupt rejected");
        let msg = err.to_string();
        assert!(msg.contains("frame corrupt"), "{msg}");
        assert!(msg.contains("deadbeef"), "{msg}");
        // Sanity: the honest frame passes the same verifier.
        assert_eq!(verify_frame(1, fnv1a64(payload), payload).unwrap(), payload);
    }

    #[test]
    fn oversized_length_prefix_is_rejected_before_allocation() {
        // Both branches of the guard: declared-too-big and
        // declared-vs-actual mismatch.
        let err = verify_frame(MAX_FRAME_BYTES + 1, 0, &[]).expect_err("oversize rejected");
        assert!(
            matches!(err, TransportError::FrameTooLarge { len, .. } if len == MAX_FRAME_BYTES + 1),
            "{err}"
        );
        let err = verify_frame(4, fnv1a64(b"hello"), b"hello!").expect_err("mismatch rejected");
        assert!(matches!(err, TransportError::FrameTooLarge { .. }), "{err}");
    }

    #[test]
    fn recv_timeout_fires_when_peer_is_silent() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _link = server.accept(T).expect("accept");
                accepted_tx.send(()).expect("report accepted");
                // Silent on purpose — but the link must stay ALIVE until
                // the client finished, or its teardown would race the
                // client's short recv (deterministic, not timed).
                done_rx.recv().expect("client done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let mut link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            accepted_rx.recv().expect("server accepted");
            let err = link
                .recv_frame(Duration::from_millis(150))
                .expect_err("timeout");
            assert!(
                matches!(err, TransportError::Timeout { timeout_ms: 150 }),
                "{err}"
            );
            done_tx.send(()).expect("signal done");
        });
    }

    #[test]
    fn send_over_max_frame_size_is_rejected_locally() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let _link = server.accept(T).expect("accept");
                accepted_tx.send(()).expect("report accepted");
                done_rx.recv().expect("client done");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let mut link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            accepted_rx.recv().expect("server accepted");
            let big = vec![0u8; (MAX_FRAME_BYTES as usize) + 1];
            let err = link.send_frame(&big, T).expect_err("local size check");
            assert!(matches!(err, TransportError::FrameTooLarge { .. }), "{err}");
            done_tx.send(()).expect("signal done");
        });
    }

    #[test]
    fn close_is_observed_by_the_peer() {
        let cert = LoopbackCert::generate().expect("cert");
        let server =
            QuicEndpoint::server(SocketAddr::from(([127, 0, 0, 1], 0)), &cert).expect("bind");
        let addr = server.local_addr();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let mut link = server.accept(T).expect("accept");
                accepted_tx.send(()).expect("report accepted");
                // Client closes; our next recv must fail with a
                // connection-level error, not hang.
                let err = link.recv_frame(T).expect_err("connection closed");
                // Either surface is a valid observation of the close:
                // the stream read fails with ConnectionLost, or the
                // connection-level error arrives first.
                let observed_close = match &err {
                    TransportError::Connection(_) => true,
                    TransportError::Read(e) => {
                        matches!(
                            e,
                            quinn::ReadExactError::ReadError(quinn::ReadError::ConnectionLost(_))
                        )
                    }
                    _ => false,
                };
                assert!(observed_close, "{err}");
            });
            let client = QuicEndpoint::client(&cert).expect("client bind");
            let mut link = client
                .connect(addr, LOOPBACK_SERVER_NAME, T)
                .expect("connect");
            accepted_rx.recv().expect("server accepted");
            link.close(42, "test-close");
        });
    }
}
