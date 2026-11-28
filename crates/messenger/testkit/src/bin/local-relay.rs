// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A relay on the loopback for a check by hand: the `LocalRelay` of the
//! tests as a process. It prints its `ws://` URL on stdout and listens
//! until Ctrl-C (or SIGTERM). Nothing is kept: a restart is an empty relay.
//!
//! ```text
//! local-relay            prints ws://127.0.0.1:<port> and serves
//! ```
//!
//! `cargo run -p messenger-testkit --no-default-features --bin local-relay`
//! (the engine of calls is not needed).

#[tokio::main]
async fn main() {
    let relay = messenger_testkit::local_relay().await;
    let url = relay.url().await;
    println!("{}", url.as_str_without_trailing_slash());
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("a SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    drop(relay);
}
