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
//! Every connection, either way, is kept on a list ([`Sockets`]): after the
//! device slept, a socket may look open and carry nothing, and only a ping
//! tells. One that stays silent is closed, and `nostr-sdk` connects anew.
//!
//! The address is not logged anywhere here: it may carry the relay's key.

use std::fmt;
use std::future::{poll_fn, Future};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{ready, Context, Poll};
use std::time::Duration;

use async_wsocket::message::CloseFrame;
use async_wsocket::Message;
use futures_util::stream::SplitSink;
use futures_util::{Sink, SinkExt, StreamExt};
use messenger_vlink::{Net, Route};
use nostr_sdk::error::Error;
use nostr_sdk::transport::websocket::{DefaultWebsocketTransport, WebSocketSink, WebSocketStream, WebSocketTransport};
use nostr::types::Url;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const USER_AGENT: &str = "veydan-messenger";

/// The largest message of a relay that is read, other than negentropy:
/// twice the largest event the relays of the project store (`maxEventSize`
/// of strfry, 128 KiB), room for the wrapping. A larger one is dropped, the
/// connection stays: an event a relay of somebody's own keeps over the
/// limit would come back on every `REQ`, and a closed connection would only
/// bring it again.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// The largest negentropy message (`NEG-MSG`, `NEG-ERR`): a device with an
/// empty or old set gets one list of every id of a range (64 hex each),
/// and strfry cuts such a frame at about 500 000 bytes of its own, 1 MB in
/// hex.
pub const MAX_NEGENTROPY_BYTES: usize = 2 * 1024 * 1024;

/// What a socket opened here (through a bridge) reads at most, a frame or a
/// message: past it the connection ends. Above [`MAX_NEGENTROPY_BYTES`], so
/// that what the limits drop is dropped here as on the direct way, and the
/// connection ends only on what is no message of a relay at all. A bridge
/// carries the relays of the project alone, whose events are 128 KiB at most.
pub const MAX_BRIDGED_READ_BYTES: usize = 4 * 1024 * 1024;
const _: () = assert!(MAX_BRIDGED_READ_BYTES > MAX_NEGENTROPY_BYTES, "what the limits drop is read first");

/// Connects relays the way the rule `net` says.
#[derive(Clone)]
pub struct BridgedTransport {
    net: Net,
    sockets: Arc<Sockets>,
}

impl BridgedTransport {
    pub fn new(net: Net) -> Self {
        Self { net, sockets: Arc::default() }
    }

    /// The connections this transport opened and that are still open.
    pub fn sockets(&self) -> Arc<Sockets> {
        self.sockets.clone()
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
        Box::pin(async move {
            let halves = match url.host_str() {
                Some(host) if self.net.route(host) == Route::Bridge => self.through_bridge(url, host).await?,
                _ => DefaultWebsocketTransport.connect(url, proxy).await?,
            };
            Ok(self.sockets.keep(halves))
        })
    }
}

/// The payload of the pings sent from here: their pongs are not handed to
/// `nostr-sdk`, which takes a pong it did not ask for as an error.
const PROBE: &[u8] = b"veydan-alive?";

/// The connections open now.
#[derive(Default)]
pub struct Sockets {
    open: Mutex<Vec<Weak<Socket>>>,
}

struct Socket {
    sink: Mutex<WebSocketSink>,
    /// Messages received, of any kind: a sign of life.
    heard: AtomicU64,
    closed: AtomicBool,
    close: Notify,
}

impl Socket {
    fn shut(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.close.notify_one();
    }

    fn gone() -> Error {
        Error::transport("the connection was closed: it went silent, or its way changed")
    }

    async fn ping(&self) -> Result<(), Error> {
        poll_fn(|cx| {
            let mut sink = self.sink.lock().expect("socket sink");
            ready!(sink.as_mut().poll_ready(cx))?;
            Poll::Ready(sink.as_mut().start_send(Message::Ping(PROBE.to_vec())))
        })
        .await?;
        poll_fn(|cx| self.sink.lock().expect("socket sink").as_mut().poll_flush(cx)).await
    }
}

impl Sockets {
    fn live(&self) -> Vec<Arc<Socket>> {
        let mut open = self.open.lock().expect("sockets");
        open.retain(|s| s.strong_count() > 0);
        open.iter().filter_map(Weak::upgrade).collect()
    }

    /// Puts a new connection on the list and gives back its halves, which
    /// end when the connection is closed from here.
    fn keep(&self, (sink, stream): (WebSocketSink, WebSocketStream)) -> (WebSocketSink, WebSocketStream) {
        let socket = Arc::new(Socket {
            sink: Mutex::new(sink),
            heard: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            close: Notify::new(),
        });
        {
            let mut open = self.open.lock().expect("sockets");
            open.retain(|s| s.strong_count() > 0);
            open.push(Arc::downgrade(&socket));
        }
        let sink: WebSocketSink = Box::pin(Shared(socket.clone()));
        let stream = futures_util::stream::unfold(Some((stream, socket)), |state| async move {
            let (mut stream, socket) = state?;
            loop {
                tokio::select! {
                    biased;
                    _ = socket.close.notified() => return Some((Err(Socket::gone()), None)),
                    item = stream.next() => {
                        let item = item?;
                        socket.heard.fetch_add(1, Ordering::SeqCst);
                        if matches!(&item, Ok(Message::Pong(payload)) if payload.as_slice() == PROBE) {
                            continue;
                        }
                        if let Some((size, limit)) = item.as_ref().ok().and_then(oversized) {
                            tracing::warn!(size, limit, "a relay sent a message over the limit: it is dropped");
                            continue;
                        }
                        return Some((item, Some((stream, socket))));
                    }
                }
            }
        });
        (sink, Box::pin(stream))
    }

    /// Asks every open connection for a sign of life and closes the ones
    /// that give none within `patience`. Returns how many were closed.
    pub async fn probe(&self, patience: Duration) -> usize {
        let checks = self.live().into_iter().map(|socket| async move {
            let before = socket.heard.load(Ordering::SeqCst);
            let deadline = tokio::time::Instant::now() + patience;
            let pinged = tokio::time::timeout_at(deadline, socket.ping()).await;
            if matches!(pinged, Ok(Ok(()))) {
                while tokio::time::Instant::now() < deadline {
                    if socket.heard.load(Ordering::SeqCst) != before {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            socket.shut();
            true
        });
        futures_util::future::join_all(checks).await.into_iter().filter(|closed| *closed).count()
    }

    /// Closes every open connection: they are made anew, each the way the
    /// rule says now.
    pub fn close_all(&self) {
        for socket in self.live() {
            socket.shut();
        }
    }

    pub fn count(&self) -> usize {
        self.live().len()
    }
}

/// The size of a message past its limit, and the limit: a negentropy
/// message has [`MAX_NEGENTROPY_BYTES`], any other [`MAX_MESSAGE_BYTES`].
/// The transport of `nostr-sdk` (the direct way) reads a message whole
/// under the limits of its library, so the cut is made here, before the
/// message is parsed; a socket opened here (through a bridge) reads no
/// message past [`MAX_BRIDGED_READ_BYTES`] at all.
fn oversized(message: &Message) -> Option<(usize, usize)> {
    let (size, limit) = match message {
        Message::Text(text) if negentropy(text) => (text.len(), MAX_NEGENTROPY_BYTES),
        Message::Text(text) => (text.len(), MAX_MESSAGE_BYTES),
        Message::Binary(data) => (data.len(), MAX_MESSAGE_BYTES),
        _ => return None,
    };
    (size > limit).then_some((size, limit))
}

/// `["NEG-…", …]`: what a relay answers to `NEG-OPEN` and `NEG-MSG`.
fn negentropy(text: &str) -> bool {
    let rest = text.trim_start();
    rest.strip_prefix('[').is_some_and(|r| r.trim_start().starts_with("\"NEG-"))
}

/// What a socket opened here reads at most: a frame or a message past it
/// ends the connection with an error.
fn limits() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_BRIDGED_READ_BYTES)).max_frame_size(Some(MAX_BRIDGED_READ_BYTES))
}

/// The sink of a kept connection, shared with [`Sockets::probe`], which
/// sends its pings through it.
struct Shared(Arc<Socket>);

impl Sink<Message> for Shared {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(Socket::gone()));
        }
        self.0.sink.lock().expect("socket sink").as_mut().poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Error> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Err(Socket::gone());
        }
        self.0.sink.lock().expect("socket sink").as_mut().start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Poll::Ready(Err(Socket::gone()));
        }
        self.0.sink.lock().expect("socket sink").as_mut().poll_flush(cx)
    }

    // A connection closed from here is not waited on: a dead one would hold
    // the next attempt back for the whole timeout of `nostr-sdk`.
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.0.closed.load(Ordering::SeqCst) {
            return Poll::Ready(Ok(()));
        }
        self.0.sink.lock().expect("socket sink").as_mut().poll_close(cx)
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
            let (socket, _) = tokio_tungstenite::client_async_with_config(request, stream, Some(limits())).await.map_err(Error::transport)?;
            return Ok(halves(socket));
        }
        // The relay's own certificate is checked here, end to end: what the
        // bridge and the hub carry is this TLS, which they cannot open.
        let config = messenger_http::tls_config().map_err(Error::transport)?;
        let name = ServerName::try_from(host.to_string()).map_err(Error::transport)?;
        let tls = TlsConnector::from(Arc::new(config)).connect(name, stream).await.map_err(Error::transport)?;
        let (socket, _) = tokio_tungstenite::client_async_with_config(request, tls, Some(limits())).await.map_err(Error::transport)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(messages: Vec<Message>) -> (Arc<Sockets>, WebSocketStream) {
        let sockets = Arc::new(Sockets::default());
        let sink: WebSocketSink = Box::pin(futures_util::sink::drain().sink_map_err(|e: std::convert::Infallible| match e {}));
        let stream: WebSocketStream = Box::pin(futures_util::stream::iter(messages.into_iter().map(Ok)));
        let (_, stream) = sockets.keep((sink, stream));
        (sockets, stream)
    }

    /// `["EVENT","s",{"content":"xxx…"}]` of `size` bytes.
    fn event_of(size: usize) -> String {
        let head = r#"["EVENT","s",{"content":""#;
        let tail = r#""}]"#;
        format!("{head}{}{tail}", "x".repeat(size - head.len() - tail.len()))
    }

    /// `["NEG-MSG","s","…"]` of `size` bytes, hex as a relay sends it.
    fn neg_of(size: usize) -> String {
        let head = r#"[ "NEG-MSG","s",""#;
        let tail = r#""]"#;
        format!("{head}{}{tail}", "ab".repeat((size - head.len() - tail.len()) / 2))
    }

    fn text(item: Option<Result<Message, Error>>) -> String {
        match item {
            Some(Ok(Message::Text(t))) => t,
            other => panic!("expected a text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_event_over_the_limit_is_dropped_and_the_connection_stays() {
        let fits = event_of(MAX_MESSAGE_BYTES);
        let (sockets, mut stream) = kept(vec![
            Message::Text(fits.clone()),
            Message::Text(event_of(300 * 1024)),
            Message::Binary(vec![0; MAX_MESSAGE_BYTES + 1]),
            Message::Text("after".into()),
        ]);
        assert_eq!(text(stream.next().await), fits, "a message at the limit passes");
        assert_eq!(text(stream.next().await), "after", "the one past it is dropped, the next one comes");
        assert!(stream.next().await.is_none());
        assert!(sockets.live().iter().all(|s| !s.closed.load(Ordering::SeqCst)), "nothing was closed");
    }

    #[tokio::test]
    async fn a_negentropy_list_of_a_megabyte_passes() {
        let list = neg_of(1024 * 1024);
        assert!(negentropy(&list));
        let (_, mut stream) = kept(vec![Message::Text(list.clone()), Message::Text(neg_of(MAX_NEGENTROPY_BYTES + 2)), Message::Text("after".into())]);
        assert_eq!(text(stream.next().await).len(), list.len(), "a NEG-MSG of 1 MiB is read");
        assert_eq!(text(stream.next().await), "after", "past its own limit a NEG-MSG is dropped too");
        assert!(!negentropy(&event_of(100)) && !negentropy(r#"["EVENT","NEG-"]"#));
        assert_eq!(oversized(&Message::Ping(vec![0; MAX_MESSAGE_BYTES + 1])), None, "control frames are the library's");
    }

    #[test]
    fn a_socket_opened_here_reads_past_the_limits_and_no_further() {
        let config = limits();
        assert_eq!(config.max_message_size, Some(MAX_BRIDGED_READ_BYTES));
        assert_eq!(config.max_frame_size, Some(MAX_BRIDGED_READ_BYTES));
    }
}
