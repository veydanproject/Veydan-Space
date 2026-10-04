// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The websocket of a relay, through a bridge when the rule says so.
//!
//! `nostr-sdk` opens its websockets itself and offers one seam: a
//! transport that is handed the address and gives back the two halves of a
//! connection. This transport asks `messenger-vlink` which way the relay is
//! reached:
//!
//! - directly: the transport of `nostr-sdk` does it, as it always did;
//! - through a bridge: the stream comes from the bridge, TLS to the relay
//!   is spoken over it with the same roots as every HTTPS request of the
//!   messenger, and the websocket is opened on top.
//!
//! The address is not logged anywhere here: it may carry the relay's key.

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_wsocket::message::CloseFrame;
use async_wsocket::Message;
use futures_util::stream::SplitSink;
use futures_util::{Sink, SinkExt, StreamExt};
use messenger_vlink::{Net, Route};
use nostr_sdk::error::Error;
use nostr_sdk::transport::websocket::{DefaultWebsocketTransport, WebSocketSink, WebSocketStream, WebSocketTransport};
use nostr::types::Url;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const USER_AGENT: &str = "veydan-messenger";

/// Connects relays the way the rule `net` says.
#[derive(Clone)]
pub struct BridgedTransport {
    net: Net,
}

impl BridgedTransport {
    pub fn new(net: Net) -> Self {
        Self { net }
    }
}

/// Follows the rule of the process.
impl Default for BridgedTransport {
    fn default() -> Self {
        Self::new(Net::global().clone())
    }
}

impl fmt::Debug for BridgedTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgedTransport").field("active", &self.net.is_active()).finish()
    }
}

type Connected = Result<(WebSocketSink, WebSocketStream), Error>;

impl WebSocketTransport for BridgedTransport {
    fn support_ping(&self) -> bool {
        true
    }

    fn connect<'a>(
        &'a self,
        url: &'a Url,
        proxy: Option<SocketAddr>,
    ) -> Pin<Box<dyn Future<Output = Connected> + Send + 'a>> {
        match url.host_str() {
            Some(host) if self.net.route(host) == Route::Bridge => Box::pin(self.through_bridge(url, host)),
            _ => DefaultWebsocketTransport.connect(url, proxy),
        }
    }
}

impl BridgedTransport {
    async fn through_bridge(&self, url: &Url, host: &str) -> Connected {
        let secure = match url.scheme() {
            "wss" => true,
            "ws" => false,
            other => return Err(Error::transport(format!("not a websocket address: {other}"))),
        };
        let port = url.port_or_known_default().unwrap_or(if secure { 443 } else { 80 });
        let mut request = url.as_str().into_client_request().map_err(Error::transport)?;
        request.headers_mut().insert("user-agent", HeaderValue::from_static(USER_AGENT));

        let stream = self.net.open(host, port).await.map_err(Error::transport)?;
        if !secure {
            let (socket, _) = tokio_tungstenite::client_async(request, stream).await.map_err(Error::transport)?;
            return Ok(halves(socket));
        }
        // The relay's own certificate is checked here, end to end: what the
        // bridge and the hub carry is this TLS, which they cannot open.
        let config = messenger_http::tls_config().map_err(Error::transport)?;
        let name = ServerName::try_from(host.to_string()).map_err(Error::transport)?;
        let tls = TlsConnector::from(Arc::new(config)).connect(name, stream).await.map_err(Error::transport)?;
        let (socket, _) = tokio_tungstenite::client_async(request, tls).await.map_err(Error::transport)?;
        Ok(halves(socket))
    }
}

/// The two halves `nostr-sdk` wants, speaking its message type.
fn halves<S>(socket: tokio_tungstenite::WebSocketStream<S>) -> (WebSocketSink, WebSocketStream)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = socket.split();
    let sink: WebSocketSink = Box::pin(Outgoing(tx));
    let stream: WebSocketStream = Box::pin(rx.filter_map(|item| async move {
        match item {
            Ok(message) => incoming(message).map(Ok),
            Err(e) => Some(Err(Error::transport(e))),
        }
    }));
    (sink, stream)
}

/// A message of the socket as `nostr-sdk` names it. A raw frame is never
/// handed out by a socket that reads whole messages.
fn incoming(message: WsMessage) -> Option<Message> {
    Some(match message {
        WsMessage::Text(text) => Message::Text(text.to_string()),
        WsMessage::Binary(data) => Message::Binary(data.to_vec()),
        WsMessage::Ping(data) => Message::Ping(data.to_vec()),
        WsMessage::Pong(data) => Message::Pong(data.to_vec()),
        WsMessage::Close(frame) => {
            Message::Close(frame.map(|f| CloseFrame { code: f.code.into(), reason: f.reason.to_string() }))
        }
        WsMessage::Frame(_) => return None,
    })
}

// Written out, not `sink_map_err`: that adapter panics when polled after an
// error, and `nostr-sdk` does poll (its issue 984).
struct Outgoing<S>(SplitSink<tokio_tungstenite::WebSocketStream<S>, WsMessage>);

impl<S: AsyncRead + AsyncWrite + Unpin> Sink<Message> for Outgoing<S> {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.0.poll_ready_unpin(cx).map_err(Error::transport)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Error> {
        self.0.start_send_unpin(item.into()).map_err(Error::transport)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.0.poll_flush_unpin(cx).map_err(Error::transport)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.0.poll_close_unpin(cx).map_err(Error::transport)
    }
}
