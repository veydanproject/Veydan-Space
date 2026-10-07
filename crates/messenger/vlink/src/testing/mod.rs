// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A bridge for tests: what a real one looks like to a client, on this
//! machine. Real TLS and real h2; behind it, instead of a hub, a plain
//! connection to one local port.
//!
//! Compiled only with the `testing` feature, which the tests of this crate
//! and of the crates that go through a bridge turn on. The certificates in
//! `certs.rs` were made for the tests and guard nothing.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http::{Method, Response, StatusCode};
use crate::proto::io::{self, relay};
use crate::proto::pin;
use crate::{BridgeId, BridgeRef, H2Stream};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

mod certs;
pub use certs::{CERT, KEY, OTHER_CERT, OTHER_KEY, RENEWED_CERT, RENEWED_KEY};

/// What the bridge does with a stream.
#[derive(Clone, Copy)]
pub enum Behaviour {
    /// Carries it to this address, whatever host was named.
    CarryTo(SocketAddr),
    /// Answers every stream with this status.
    Answer(StatusCode),
}

pub struct FakeBridge {
    pub bridge: BridgeRef,
    /// Hosts the streams named, in order.
    pub asked: Arc<std::sync::Mutex<Vec<String>>>,
    pub streams: Arc<AtomicUsize>,
}

/// The certificates of a chain, in the order the bridge shows them.
fn chain(chain_pem: &str) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(chain_pem.as_bytes()).collect::<Result<_, _>>().unwrap()
}

/// The id of the bridge whose chain this is: of the key that signed the
/// TLS certificate, carried by the second certificate.
pub fn id_of(chain_pem: &str) -> BridgeId {
    BridgeId::of_cert(chain(chain_pem)[1].as_ref()).unwrap()
}

/// A bridge with the usual chain and TLS key (`CERT`, `KEY`).
pub async fn bridge(behaviour: Behaviour) -> FakeBridge {
    bridge_with(behaviour, CERT, KEY).await
}

/// A bridge showing the chain `chain_pem` over the TLS key `key_pem`; its
/// reference carries the id of the key that signed the chain.
pub async fn bridge_with(behaviour: Behaviour, chain_pem: &str, key_pem: &str) -> FakeBridge {
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(pin::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain(chain_pem), key)
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let streams = Arc::new(AtomicUsize::new(0));

    let (asked_in, streams_in) = (asked.clone(), streams.clone());
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (acceptor, asked, streams) = (acceptor.clone(), asked_in.clone(), streams_in.clone());
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else { return };
                let Ok(mut connection) = io::server_builder().handshake::<_, Bytes>(tls).await else { return };
                while let Some(Ok((request, mut respond))) = connection.accept().await {
                    let (asked, streams) = (asked.clone(), streams.clone());
                    tokio::spawn(async move {
                        assert_eq!(request.method(), Method::CONNECT);
                        let authority = request.uri().authority().unwrap().to_string();
                        asked.lock().unwrap().push(authority);
                        let refuse = |respond: &mut h2::server::SendResponse<Bytes>, status| {
                            let _ = respond.send_response(Response::builder().status(status).body(()).unwrap(), true);
                        };
                        let to = match behaviour {
                            Behaviour::Answer(status) => return refuse(&mut respond, status),
                            Behaviour::CarryTo(to) => to,
                        };
                        let Ok(mut server) = TcpStream::connect(to).await else {
                            return refuse(&mut respond, StatusCode::BAD_GATEWAY);
                        };
                        let ok = Response::builder().status(StatusCode::OK).body(()).unwrap();
                        let Ok(send) = respond.send_response(ok, false) else { return };
                        streams.fetch_add(1, Ordering::Relaxed);
                        let mut stream = H2Stream::new(send, request.into_body());
                        let _ = relay(&mut stream, &mut server).await;
                    });
                }
            });
        }
    });
    FakeBridge { bridge: BridgeRef { addr, id: id_of(chain_pem), sni: None }, asked, streams }
}

/// A server that sends back what it gets.
pub async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
                let _ = write.shutdown().await;
            });
        }
    });
    addr
}

/// Says `text` into `stream` and reads the same number of bytes back.
pub async fn there_and_back<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(stream: &mut S, text: &str) {
    stream.write_all(text.as_bytes()).await.unwrap();
    let mut back = vec![0u8; text.len()];
    stream.read_exact(&mut back).await.unwrap();
    assert_eq!(back, text.as_bytes());
}
