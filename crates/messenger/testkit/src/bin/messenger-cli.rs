// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Headless messenger over `MessengerRuntime`. Second participant for smoke
//! tests and living proof that the runtime works without a UI.
//!
//! ```text
//! messenger-cli [--data-dir DIR] keygen [--password PW]
//! messenger-cli [--data-dir DIR] import <nsec|ncryptsec> <secret> [--password PW]
//! messenger-cli [--data-dir DIR] whoami
//! messenger-cli [--data-dir DIR] relays
//! messenger-cli [--data-dir DIR] relay-add <wss-url> [--key API_KEY]
//! messenger-cli [--data-dir DIR] send <npub|hex> <text…>
//! messenger-cli [--data-dir DIR] tail
//! messenger-cli [--data-dir DIR] notify-tail
//! messenger-cli [--data-dir DIR] sync [SECONDS]
//! messenger-cli [--data-dir DIR] chats
//! messenger-cli [--data-dir DIR] history <npub|hex>
//! messenger-cli [--data-dir DIR] edit <message-id> <text…>
//! messenger-cli [--data-dir DIR] delete <message-id>
//! messenger-cli [--data-dir DIR] relation <npub|hex>
//! messenger-cli [--data-dir DIR] request|accept|decline|block|unblock|remove <npub|hex>
//! messenger-cli [--data-dir DIR] push-on <token> [--server URL]
//! messenger-cli [--data-dir DIR] push-status | push-test | push-off
//! messenger-cli [--data-dir DIR] servers [veydan|own|refresh]
//! messenger-cli [--data-dir DIR] net [off|on|auto|check|add <address:port#id>|remove <id>]
//! messenger-cli manifest-keygen <secret-file>
//! messenger-cli manifest-sign --key-file <secret-file> <doc.json> <signed.json>
//! ```
//!
//! Secrets live in `<data-dir>/secrets.json` in plaintext: development only.

use messenger_core::MessengerConfig;
use messenger_runtime::MessengerRuntime;
use messenger_testkit::FileSecretStore;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "usage: messenger-cli [--data-dir DIR] <keygen [--password PW] | import <nsec|ncryptsec> <secret> [--password PW] \
         | whoami | relays | relay-add <url> [--key K] | send <to> <text…> | tail | notify-tail | sync [secs] | chats | history <peer> | shared <peer|group:id> [visual|files|links|voice] | edit <id> <text…> | delete <id> | relation <peer> | request|accept|decline|block|unblock|remove <peer> | push-on <token> [--server URL] | push-status | push-test | push-off | profile-set <name> [picture] | wrap <to|group:ID|stranger:ID> <text…> [--send] | notify-describe <event.json> [--type dm|group] [--group ID] | servers [veydan|own|refresh] | net [off|on|auto|check|add <bridge>|remove <id>] | manifest-keygen <file> | manifest-sign --key-file F <doc.json> <signed.json>>"
    );
    std::process::exit(2)
}

fn take_flag(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    if i + 1 >= args.len() {
        usage();
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = take_flag(&mut args, "--data-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./messenger-cli-data"));
    let password = take_flag(&mut args, "--password");
    let api_key = take_flag(&mut args, "--key");
    if args.is_empty() {
        usage();
    }
    let cmd = args.remove(0);
    let key_file = take_flag(&mut args, "--key-file");

    // Operator tools: no messenger data involved.
    match cmd.as_str() {
        "manifest-keygen" => return manifest_keygen(args.first().unwrap_or_else(|| usage())),
        "manifest-sign" => {
            let (Some(key), [input, output]) = (key_file, args.as_slice()) else { usage() };
            return manifest_sign(&key, input, output);
        }
        _ => {}
    }

    let config = MessengerConfig::new(data_dir.clone());
    let secrets = Arc::new(FileSecretStore::open(data_dir.join("secrets.json")).await.unwrap_or_else(die));
    let rt = MessengerRuntime::start(config, secrets).await.unwrap_or_else(die);

    // Scenarios written before the choice existed expect the project's servers.
    if cmd != "servers" && rt.servers_mode().await.unwrap_or_else(die).is_none() {
        eprintln!("(no servers chosen: using the Veydan servers; `servers own` to change)");
        print_check(&rt.servers_use_veydan().await.unwrap_or_else(die));
    }

    match cmd.as_str() {
        "servers" => match args.first().map(String::as_str) {
            Some("veydan") => print_check(&rt.servers_use_veydan().await.unwrap_or_else(die)),
            Some("own") => {
                rt.servers_use_own().await.unwrap_or_else(die);
                println!("own servers");
            }
            Some("refresh") => match rt.manifest_refresh(true).await.unwrap_or_else(die) {
                Some(check) => print_check(&check),
                None => println!("not using the Veydan servers: nothing asked"),
            },
            None => {
                let info = rt.relays().manifest_info().await.unwrap_or_else(die);
                println!("mode      {:?}", info.mode);
                println!("manifest  #{:?} from {}", info.serial, info.origin);
                println!("checked   {:?}", info.checked_at);
            }
            _ => usage(),
        },
        "net" => match args.first().map(String::as_str) {
            None => print_net(&rt.net_status().await.unwrap_or_else(die)),
            Some(mode @ ("off" | "on" | "auto")) => {
                let mode = messenger_runtime::net::NetMode::parse(mode).expect("matched above");
                print_net(&rt.net_set_mode(mode).await.unwrap_or_else(die));
            }
            Some("check") => {
                let check = rt.net_check().await.unwrap_or_else(die);
                println!("direct    {}", if check.direct { "carries bytes" } else { "does not carry" });
                if !check.direct {
                    println!("bridge    {}", if check.bridge { "carries bytes" } else { "does not carry" });
                }
                println!("verdict   {:?}", check.verdict);
                print_net(&rt.net_status().await.unwrap_or_else(die));
            }
            Some("add") if args.len() == 2 => print_net(&rt.net_bridge_add(&args[1]).await.unwrap_or_else(die)),
            Some("remove") if args.len() == 2 => print_net(&rt.net_bridge_remove(&args[1]).await.unwrap_or_else(die)),
            _ => usage(),
        },
        "keygen" => {
            let pw = password.unwrap_or_else(|| "cli-dev-password".into());
            let created = rt.identity().create(&pw).await.unwrap_or_else(die);
            rt.refresh_signer().await.unwrap_or_else(die);
            println!("npub      {}", created.identity.npub);
            println!("pubkey    {}", created.identity.pubkey.as_hex());
            println!("ncryptsec {}", created.ncryptsec);
        }
        "import" => {
            if args.len() < 2 {
                usage();
            }
            let kind = args[0].as_str();
            let secret = args[1].as_str();
            let id = match kind {
                "nsec" => rt.identity().import_nsec(secret).await,
                "ncryptsec" => rt.identity().import_ncryptsec(secret, &password.unwrap_or_default()).await,
                _ => usage(),
            }
            .unwrap_or_else(die);
            rt.refresh_signer().await.unwrap_or_else(die);
            println!("imported {}", id.npub);
        }
        "whoami" => match rt.identity().get().await.unwrap_or_else(die) {
            Some(id) => println!("{}\n{}", id.npub, id.pubkey.as_hex()),
            None => println!("(no identity — run keygen or import)"),
        },
        "relays" => {
            wait_connect(&rt).await;
            for r in rt.relays().list().await.unwrap_or_else(die) {
                println!(
                    "{:<12} {:<9} {:<8} {} {}",
                    format!("{:?}", r.state).to_lowercase(),
                    r.source,
                    r.auth_type.unwrap_or_else(|| "-".into()),
                    if r.enabled { "on " } else { "off" },
                    r.url
                );
            }
        }
        "relay-add" => {
            if args.is_empty() {
                usage();
            }
            let v = rt.relays().add_user(&args[0], api_key).await.unwrap_or_else(die);
            println!("added {} ({})", v.url, v.auth_type.unwrap_or_else(|| "no auth".into()));
        }
        "send" => {
            if args.len() < 2 {
                usage();
            }
            let to = args.remove(0);
            let text = args.join(" ");
            wait_connect(&rt).await;
            let id = rt.dm_send_text(&to, &text, None).await.unwrap_or_else(die).id;
            // Give the outbox pump a moment to report.
            flush(&rt).await;
            let pending = rt.outbox().pending().await.unwrap_or(0);
            println!("queued {id} (pending in outbox: {pending})");
        }
        "tail" => {
            wait_connect(&rt).await;
            let st = rt.status().await.unwrap_or_else(die);
            eprintln!(
                "listening as session={} relays={}/{} — Ctrl-C to stop",
                st.session_active, st.relays_connected, st.relays_total
            );
            let mut rx = rt.ui_events();
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Ok(e) => println!("{} {}", e.name, e.payload),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                        Err(_) => break,
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        }
        "notify-tail" => {
            // What a computer would show for each live message: the runtime's
            // `notify`, told the way a push is (`live_notice`).
            wait_connect(&rt).await;
            eprintln!("listening for notices — Ctrl-C to stop");
            let mut rx = rt.ui_events();
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Ok(e) if e.name == "notify" => {
                            let notice: messenger_core::Notice = serde_json::from_value(e.payload).unwrap_or_else(|e| die(messenger_core::MessengerError::Invalid(e.to_string())));
                            let outcome = rt.live_notice(&notice, false).await.unwrap_or_else(die);
                            println!("{}", serde_json::to_string(&outcome).unwrap());
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                        Err(_) => break,
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        }
        "chats" => {
            for c in rt.dm().list_chats(true).await.unwrap_or_else(die) {
                println!(
                    "{}  unread={} mode={}  {}  | {}",
                    c.peer_npub.unwrap_or(c.id),
                    c.unread,
                    c.mode,
                    c.title,
                    c.last_preview.unwrap_or_default()
                );
            }
        }
        "history" => {
            if args.is_empty() {
                usage();
            }
            let chat = rt.chat_open(&args[0]).await.unwrap_or_else(die);
            for m in rt.dm().messages(&chat.id, None, 200).await.unwrap_or_else(die) {
                println!(
                    "{} {} {:<8} {}{}{}",
                    m.created_at,
                    if m.direction == "out" { "->" } else { "<-" },
                    m.status,
                    if m.deleted { "(deleted)".to_string() } else { m.text.clone().unwrap_or_else(|| format!("[{}]", m.content_type)) },
                    if m.edited_at.is_some() && !m.deleted { " (edited)" } else { "" },
                    format_args!("  #{}", &m.id[..8.min(m.id.len())]),
                );
            }
        }
        "shared" => {
            if args.is_empty() {
                usage();
            }
            let chat_id = if args[0].starts_with("group:") { args[0].clone() } else { rt.chat_open(&args[0]).await.unwrap_or_else(die).id };
            let c = rt.shared_counts(&chat_id).await.unwrap_or_else(die);
            println!("visual={} files={} links={} voice={}", c.visual, c.files, c.links, c.voice);
            if let Some(word) = args.get(1) {
                let section: messenger_runtime::SharedSection =
                    serde_json::from_value(serde_json::Value::String(word.clone())).unwrap_or_else(|_| usage());
                for m in rt.shared(&chat_id, section, None, 200).await.unwrap_or_else(die) {
                    let media = m.media.as_ref().map(|v| format!("[{} {}] ", v["kind"].as_str().unwrap_or("?"), v["name"].as_str().unwrap_or(""))).unwrap_or_default();
                    println!("{} {}{}  #{}", m.created_at, media, m.text.unwrap_or_default(), &m.id[..8.min(m.id.len())]);
                }
            }
        }
        "edit" => {
            if args.len() < 2 {
                usage();
            }
            let id = full_id(&rt, &args.remove(0)).await;
            wait_connect(&rt).await;
            let m = rt.dm_edit(&id, &args.join(" ")).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("edited {} -> {}", m.id, m.text.unwrap_or_default());
        }
        "delete" => {
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            wait_connect(&rt).await;
            rt.dm_delete(&id, true).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("deleted {id}");
        }
        "sync" => {
            // Stay online for a while so live delivery and the history
            // catch-up can finish, then report.
            let secs: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(8);
            wait_connect(&rt).await;
            tokio::time::sleep(Duration::from_secs(secs)).await;
            let st = rt.status().await.unwrap_or_else(die);
            println!(
                "received={} dm={} duplicates={} outbox_pending={}",
                st.ingress.received, st.ingress.dm, st.ingress.duplicates, st.outbox_pending
            );
        }
        "outbox" => {
            for r in messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap_or_else(die) {
                let op = r.outbound_json.chars().take(60).collect::<String>();
                println!("{} attempts={} error={} {}", r.state, r.attempts, r.last_error.unwrap_or_default(), op);
            }
        }
        "relation" => {
            if args.is_empty() {
                usage();
            }
            let r = rt.dm_relation(&args[0]).await.unwrap_or_else(die);
            println!(
                "mode={} my_contact={} blocked={} peer_signal={} mutual={} can_send={}",
                r.mode, r.my_contact, r.blocked, r.peer_signal, r.was_ever_mutual, r.can_send
            );
        }
        "request" | "accept" | "decline" | "block" | "unblock" | "remove" => {
            if args.is_empty() {
                usage();
            }
            let action = match cmd.as_str() {
                "request" => messenger_runtime::DmAction::Request,
                "accept" => messenger_runtime::DmAction::Accept,
                "decline" => messenger_runtime::DmAction::Decline,
                "block" => messenger_runtime::DmAction::Block,
                "unblock" => messenger_runtime::DmAction::Unblock,
                _ => messenger_runtime::DmAction::Remove,
            };
            wait_connect(&rt).await;
            let r = rt.dm_act(&args[0], action).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("mode={} can_send={}", r.mode, r.can_send);
        }
        "media-servers" => {
            for s in rt.media_servers().await.unwrap_or_else(die) {
                println!(
                    "{:<8} {} {}  access={} secret={}  {}  [{}]",
                    s.kind,
                    if s.enabled { "on " } else { "off" },
                    s.public_base,
                    s.access_key.unwrap_or_else(|| "-".into()),
                    if s.has_secret { "set" } else { "-" },
                    s.source,
                    s.id
                );
            }
        }
        "media-s3" => {
            // media-s3 <id> <endpoint> <bucket> <access-key> <secret-key> [region]
            if args.len() < 5 {
                usage();
            }
            let v = rt
                .media_server_put(messenger_runtime::MediaServerInput {
                    id: Some(args[0].clone()),
                    kind: "s3".into(),
                    url: args[1].clone(),
                    bucket: Some(args[2].clone()),
                    access_key: Some(args[3].clone()),
                    secret_key: Some(args[4].clone()),
                    region: args.get(5).cloned(),
                    priority: None,
                    source: None,
                })
                .await
                .unwrap_or_else(die);
            println!("saved {} -> {}", v.id, v.public_base);
        }
        "media-blossom" => {
            if args.is_empty() {
                usage();
            }
            let v = rt
                .media_server_put(messenger_runtime::MediaServerInput {
                    kind: "blossom".into(),
                    url: args[0].clone(),
                    ..Default::default()
                })
                .await
                .unwrap_or_else(die);
            println!("saved {} -> {}", v.id, v.public_base);
        }
        "media-check" => {
            if args.is_empty() {
                usage();
            }
            rt.media_server_check(&args[0]).await.unwrap_or_else(die);
            println!("ok: writable and publicly readable");
        }
        "send-file" => {
            if args.len() < 2 {
                usage();
            }
            // `--batch <id>`: files sent with the same id are one album.
            let batch = args.iter().position(|a| a == "--batch").map(|i| {
                args.remove(i);
                if i < args.len() { args.remove(i) } else { usage() }
            });
            let to = args.remove(0);
            let path = PathBuf::from(args.remove(0));
            let caption = if args.is_empty() { None } else { Some(args.join(" ")) };
            wait_connect(&rt).await;
            let started = std::time::Instant::now();
            let ph = rt.dm_send_file(&to, &path, caption.as_deref(), batch.as_deref()).await.unwrap_or_else(die);
            let tid = ph.media.as_ref().and_then(|m| m["transfer_id"].as_str().map(String::from)).unwrap_or_default();
            let mut last = 0u64;
            loop {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let Some(t) = rt.media().transfer(&tid).await.unwrap_or_else(die) else { break };
                if t.done_bytes != last {
                    last = t.done_bytes;
                    eprintln!("  {} {}/{}", t.status, t.done_bytes, t.size);
                }
                if matches!(t.status.as_str(), "done" | "failed" | "cancelled" | "paused") {
                    flush(&rt).await;
                    println!(
                        "{} {} bytes in {:.1}s {}",
                        t.status,
                        t.size,
                        started.elapsed().as_secs_f32(),
                        t.failure_reason.unwrap_or_default()
                    );
                    break;
                }
            }
        }
        "download" => {
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            let started = std::time::Instant::now();
            match rt.media_download(&id, true).await.unwrap_or_else(die) {
                Some(p) => println!("{} ({:.1}s)", p.display(), started.elapsed().as_secs_f32()),
                None => println!("not downloaded"),
            }
        }
        "transfers" => {
            for t in rt.media().active_transfers().await.unwrap_or_else(die) {
                println!(
                    "{} {:<4} {:<9} {}/{} {} {}",
                    t.id,
                    t.direction,
                    t.status,
                    t.done_bytes,
                    t.size,
                    t.file_name,
                    t.failure_reason.unwrap_or_default()
                );
            }
        }
        "resume" => {
            if args.is_empty() {
                usage();
            }
            wait_connect(&rt).await;
            let started = std::time::Instant::now();
            rt.media_resume(&args[0]).await.unwrap_or_else(die);
            loop {
                flush(&rt).await;
                let Some(t) = rt.media().transfer(&args[0]).await.unwrap_or_else(die) else { break };
                if matches!(t.status.as_str(), "done" | "failed" | "cancelled" | "paused") {
                    flush(&rt).await;
                    println!(
                        "{} {}/{} in {:.1}s {}",
                        t.status,
                        t.done_bytes,
                        t.size,
                        started.elapsed().as_secs_f32(),
                        t.failure_reason.unwrap_or_default()
                    );
                    break;
                }
            }
        }
        "bench-send" => {
            // bench-send <to> [count]: how long the caller waits per message.
            if args.is_empty() {
                usage();
            }
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
            wait_connect(&rt).await;
            for i in 0..n {
                let t = std::time::Instant::now();
                let m = rt.dm_send_text(&args[0], &format!("bench {i}"), None).await.unwrap_or_else(die);
                println!("send {i}: {} ms, status on return: {}", t.elapsed().as_millis(), m.status);
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            println!("outbox pending after 3 s: {}", rt.outbox().pending().await.unwrap_or(0));
        }
        "send-voice" => {
            // send-voice <to> <audio-file> [duration-ms]: the file as a voice message.
            if args.len() < 2 {
                usage();
            }
            let bytes = std::fs::read(&args[1]).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1)
            });
            let mime = if args[1].ends_with(".ogg") { "audio/ogg;codecs=opus" } else { "audio/webm;codecs=opus" };
            wait_connect(&rt).await;
            let rec = messenger_runtime::Recording {
                kind: messenger_runtime::MediaKind::Voice,
                mime: mime.into(),
                duration_ms: args.get(2).and_then(|s| s.parse().ok()),
                waveform: Some((0..48u32).map(|i| ((i * 37) % 256) as u8).collect()),
                bytes,
            };
            let ph = rt.dm_send_recording(&args[0], rec, None).await.unwrap_or_else(die);
            let tid = ph.media.as_ref().and_then(|m| m["transfer_id"].as_str().map(String::from)).unwrap_or_default();
            for _ in 0..200 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                match rt.media().transfer(&tid).await.unwrap_or_else(die) {
                    Some(t) if matches!(t.status.as_str(), "done" | "failed" | "cancelled") => {
                        println!("{} {}", t.status, t.failure_reason.unwrap_or_default());
                        break;
                    }
                    _ => {}
                }
            }
            flush(&rt).await;
        }
        "media-info" => {
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            let m = rt.dm().message(&id).await.unwrap_or_else(die).and_then(|m| m.media).unwrap_or_default();
            for k in ["kind", "mime", "name", "size", "duration_ms"] {
                println!("{k}: {}", m.get(k).map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
            }
            println!("waveform: {} values", m.get("waveform").and_then(|w| w.as_array()).map(|a| a.len()).unwrap_or(0));
        }
        // Push notifications. The token is what a phone would bring; here it
        // is given by hand, a real one to see a push arrive, or any string
        // to see the registration alone.
        "push-on" => {
            if args.is_empty() {
                usage();
            }
            let server = take_flag(&mut args, "--server");
            let channel = messenger_runtime::push::PushChannel {
                provider: "fcm".into(),
                token: args[0].clone(),
                app_id: take_flag(&mut args, "--app").unwrap_or_else(|| "net.veydan.mobile".into()),
                app_version: Some(format!("cli {}", messenger_core::VERSION)),
            };
            rt.push_set_channel(Some(channel)).await.unwrap_or_else(die);
            if let Some(server) = server {
                rt.push_set_server(Some(server)).await.unwrap_or_else(die);
            }
            print_push(&rt.push_set_enabled(true).await.unwrap_or_else(die));
        }
        "push-status" => print_push(&rt.push_reconcile(false).await.unwrap_or_else(die)),
        "push-test" => {
            let answer = rt.push_test().await.unwrap_or_else(die);
            println!("outcome {}  trace {}", answer.outcome, answer.trace);
        }
        "push-off" => print_push(&rt.push_set_enabled(false).await.unwrap_or_else(die)),
        "profile-set" => {
            // `profile-set <name> [picture-url]`: publishes this identity's kind 0.
            if args.is_empty() {
                usage();
            }
            wait_connect(&rt).await;
            let input = messenger_runtime::ProfileInput {
                name: Some(args[0].clone()),
                picture: args.get(1).cloned(),
                ..Default::default()
            };
            let p = rt.publish_own_profile(&input).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("published {} {}", p.label(), p.picture.unwrap_or_default());
        }
        "wrap" => {
            // The gift wrap of a text to `to`, printed, not sent: what a relay
            // and a push server would see of it. `group:<id>` seals for a group;
            // `stranger:<id>` does what anybody who knows the id of a group can:
            // an event that names the group, sealed with a key the group never
            // had. `--send` also publishes a group event where the group lives.
            if args.len() < 2 {
                usage();
            }
            let send = args.iter().any(|a| a == "--send");
            args.retain(|a| a != "--send");
            let to = args.remove(0);
            let bundle = rt.notify_bundle().await.unwrap_or_else(die).unwrap_or_else(|| {
                eprintln!("error: no identity");
                std::process::exit(1)
            });
            let keys = bundle.keys().unwrap_or_else(die);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let content = messenger_core::Envelope::text(&args.join(" ")).encode();
            let group = to.strip_prefix("group:").map(|id| (id, false)).or(to.strip_prefix("stranger:").map(|id| (id, true)));
            if let Some((group_id, stranger)) = group {
                // A member seals with the group's newest key this identity holds.
                let key = if stranger {
                    messenger_groups::GroupKey::generate().unwrap_or_else(die)
                } else {
                    let entry = bundle.groups.iter().rev().find(|g| g.group_id == group_id).unwrap_or_else(|| {
                        eprintln!("error: no key of that group");
                        std::process::exit(1)
                    });
                    bundle.group_key(group_id, &entry.key_id).expect("a key of the bundle")
                };
                let signed = messenger_groups::wire::sign_message(&keys, group_id, &content, now, None).unwrap_or_else(die);
                let sealed = messenger_groups::wire::seal_message(group_id, &key, None, &signed, &keys).unwrap_or_else(die);
                println!("{}", sealed.json);
                if send {
                    let scope = messenger_core::Scope::Group { id: group_id.to_string() };
                    rt.outbox().enqueue(messenger_core::Outbound::PublishScoped { scope, event: sealed }).await.unwrap_or_else(die);
                    rt.outbox().kick();
                    flush(&rt).await;
                }
            } else {
                let to = messenger_core::PubKey::parse(&to).unwrap_or_else(|| usage());
                let wrapped = messenger_dm::wrap::wrap(&keys, &to, &content, now, None).unwrap_or_else(die);
                println!("{}", wrapped.to_peer.json);
            }
        }
        "notify-describe" => {
            // What the phone would show for this event, from this data directory.
            // `--type dm|group`, `--group <id>` as the push would name them.
            if args.is_empty() {
                usage();
            }
            let kind = take_flag(&mut args, "--type").unwrap_or_else(|| "dm".into());
            let group = take_flag(&mut args, "--group");
            let event = std::fs::read_to_string(&args[0]).unwrap_or_else(|e| {
                eprintln!("error: {}: {e}", args[0]);
                std::process::exit(1)
            });
            let mut data = std::collections::BTreeMap::new();
            data.insert("v".to_string(), "2".to_string());
            data.insert("type".to_string(), kind);
            data.insert("event".to_string(), event);
            if let Some(g) = group {
                data.insert("group_id".to_string(), g);
            }
            let push = messenger_notify::PushData::parse(&data).unwrap_or_else(die);
            let bundle = rt.notify_bundle().await.unwrap_or_else(die).unwrap_or_else(|| {
                eprintln!("error: no identity");
                std::process::exit(1)
            });
            rt.shutdown().await;
            let outcome = messenger_notify::describe(&data_dir, Some(&bundle), &push).await.unwrap_or_else(die);
            println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
            return;
        }
        "groups" => {
            for g in rt.group_list().await.unwrap_or_else(die) {
                println!(
                    "{}  {:<7} {:<10} role={:<9} members={} undecrypted={}  {}",
                    &g.id[..12],
                    g.kind,
                    g.membership,
                    g.my_role.unwrap_or_else(|| "-".into()),
                    g.members.len(),
                    g.undecrypted,
                    g.name
                );
            }
        }
        "group" => {
            if args.is_empty() {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            let g = rt.group_get(&id).await.unwrap_or_else(die);
            println!("id: {}\nname: {}\nkind: {}\nmembership: {}\ncan_post: {}\nhistory_for_new: {}", g.id, g.name, g.kind, g.membership, g.can_post, g.history_for_new);
            for m in &g.members {
                println!("member {} {}{}{}", m.pubkey, m.role, if m.muted { " muted" } else { "" }, if m.is_me { " (me)" } else { "" });
            }
            for b in &g.banned {
                println!("banned {b}");
            }
            for r in &g.requests {
                println!("request {r}");
            }
            println!("link: {}", g.link.unwrap_or_else(|| "-".into()));
        }
        "group-create" => {
            if args.len() < 2 {
                usage();
            }
            let kind = match args.remove(0).as_str() {
                "public" => messenger_runtime::GroupKind::Public,
                "private" => messenger_runtime::GroupKind::Private,
                _ => usage(),
            };
            let history = !take_switch(&mut args, "--no-history");
            wait_connect(&rt).await;
            let g = rt.group_create(kind, &args.join(" "), "", history).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("{}", g.id);
        }
        "group-invite" => {
            if args.len() < 2 {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            settle(&rt, 3).await;
            let i = rt.group_invite(&id, &args[1]).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("invited {} ({})", i.peer, i.invite_id);
        }
        "group-invites" => {
            settle(&rt, 4).await;
            for d in ["in", "out"] {
                for i in rt.group_invites(d).await.unwrap_or_else(die) {
                    println!("{} {} {} group={} peer={} | {}", d, i.invite_id, i.status, &i.group_id[..12], &i.peer[..12], i.name);
                }
            }
        }
        "group-accept" | "group-decline" => {
            if args.is_empty() {
                usage();
            }
            settle(&rt, 4).await;
            let all = rt.group_invites("in").await.unwrap_or_else(die);
            let Some(inv) = all.into_iter().find(|i| i.invite_id.starts_with(&args[0]) || i.group_id.starts_with(&args[0])) else {
                eprintln!("error: no such invitation");
                std::process::exit(1)
            };
            rt.group_answer_invite(&inv.invite_id, cmd == "group-accept").await.unwrap_or_else(die);
            flush(&rt).await;
            // The welcome comes back as soon as the inviter is online.
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            println!("{} {}", if cmd == "group-accept" { "accepted" } else { "declined" }, inv.group_id);
        }
        "group-join" => {
            if args.is_empty() {
                usage();
            }
            wait_connect(&rt).await;
            let g = rt.group_open_link(&args[0], &args[1..].join(" ")).await.unwrap_or_else(die);
            flush(&rt).await;
            tokio::time::sleep(Duration::from_secs(8)).await;
            flush(&rt).await;
            let g = rt.group_get(&g.id).await.unwrap_or_else(die);
            println!("{} {} members={}", g.id, g.membership, g.members.len());
        }
        "group-approve" | "group-reject" => {
            if args.len() < 2 {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            settle(&rt, 4).await;
            let g = rt.group_answer_request(&id, &args[1], cmd == "group-approve").await.unwrap_or_else(die);
            flush(&rt).await;
            println!("members={} requests={}", g.members.len(), g.requests.len());
        }
        "group-send" => {
            if args.len() < 2 {
                usage();
            }
            let id = group_id(&rt, &args.remove(0)).await;
            settle(&rt, 3).await;
            let m = rt.group_send_text(&id, &args.join(" "), None).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("queued {}", m.id);
        }
        "group-edit" => {
            if args.len() < 2 {
                usage();
            }
            let id = full_id(&rt, &args.remove(0)).await;
            settle(&rt, 3).await;
            let m = rt.group_edit(&id, &args.join(" ")).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("edited {} -> {}", m.id, m.text.unwrap_or_default());
        }
        "group-delete" => {
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            settle(&rt, 3).await;
            rt.group_delete(&id).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("deleted {id}");
        }
        "group-history" => {
            if args.is_empty() {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            if secs > 0 {
                settle(&rt, secs).await;
            }
            let mut list = rt.dm().messages(&format!("group:{id}"), None, 500).await.unwrap_or_else(die);
            list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            for m in list {
                println!(
                    "{} {} {:<8} {}: {}{}  #{}",
                    m.created_at,
                    if m.direction == "out" { "->" } else { "<-" },
                    m.status,
                    &m.sender_pubkey[..8],
                    if m.deleted { "(deleted)".to_string() } else if m.content_type == "system" { format!("* {}", m.text.clone().unwrap_or_default()) } else { m.text.clone().unwrap_or_else(|| format!("[{}]", m.content_type)) },
                    if m.edited_at.is_some() && !m.deleted { " (edited)" } else { "" },
                    &m.id[..8.min(m.id.len())],
                );
            }
        }
        "group-remove" | "group-ban" | "group-unban" | "group-mute" | "group-unmute" | "group-transfer" | "group-role" => {
            if args.len() < 2 {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            let who = messenger_runtime::parse_key(&args[1]).unwrap_or_else(die);
            let body = match cmd.as_str() {
                "group-remove" => messenger_runtime::GroupOp::Remove { who },
                "group-ban" => messenger_runtime::GroupOp::Ban { who },
                "group-unban" => messenger_runtime::GroupOp::Unban { who },
                "group-mute" => messenger_runtime::GroupOp::SetMuted { who, muted: true },
                "group-unmute" => messenger_runtime::GroupOp::SetMuted { who, muted: false },
                "group-transfer" => messenger_runtime::GroupOp::TransferOwnership { to: who },
                _ => {
                    let role = args.get(2).and_then(|r| messenger_runtime::GroupRole::parse(r)).unwrap_or_else(|| usage());
                    messenger_runtime::GroupOp::SetRole { who, role }
                }
            };
            settle(&rt, 3).await;
            let g = rt.group_act(&id, body).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("ok members={}", g.members.len());
        }
        "group-leave" | "group-disband" | "group-rename" | "group-link-rotate" => {
            if args.is_empty() {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            settle(&rt, 3).await;
            let g = match cmd.as_str() {
                "group-leave" => rt.group_act(&id, messenger_runtime::GroupOp::Leave).await,
                "group-disband" => rt.group_act(&id, messenger_runtime::GroupOp::Disband).await,
                "group-link-rotate" => rt.group_rotate_link(&id).await,
                _ => {
                    rt.group_act(
                        &id,
                        messenger_runtime::GroupOp::EditSettings { name: Some(args[1..].join(" ")), about: None, picture: None, history_for_new: None },
                    )
                    .await
                }
            }
            .unwrap_or_else(die);
            flush(&rt).await;
            println!("ok {} {}", g.membership, g.link.unwrap_or_default());
        }
        "group-forget" => {
            if args.is_empty() {
                usage();
            }
            let id = group_id(&rt, &args[0]).await;
            rt.group_forget(&id).await.unwrap_or_else(die);
            println!("forgotten {id}");
        }
        _ => usage(),
    }
    rt.shutdown().await;
}

async fn wait_connect(rt: &MessengerRuntime) {
    for _ in 0..40 {
        let st = rt.status().await.unwrap_or_else(die);
        if st.relays_connected > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    eprintln!("warning: no relay connected yet");
}

fn print_check(c: &messenger_runtime::ManifestCheck) {
    println!("manifest  #{} from {}{}", c.serial, c.origin, if c.updated { " (updated)" } else { "" });
    if let Some(e) = &c.error {
        println!("not fetched: {e}");
    }
}

/// New project key for signing manifests. The secret goes to `out` (0600),
/// the public half to stdout: it is what the app pins.
fn manifest_keygen(out: &str) {
    use nostr::nips::nip19::ToBech32;
    if std::path::Path::new(out).exists() {
        eprintln!("error: {out} exists; not overwriting a key");
        std::process::exit(1)
    }
    let keys = nostr::key::Keys::generate();
    let nsec = keys.secret_key().to_bech32().expect("bech32");
    write_private(out, &format!("{nsec}\n"));
    println!("pubkey {}", keys.public_key().to_hex());
    println!("secret written to {out}; keep it offline, it signs what every install trusts");
}

/// Sign the bare manifest document `input` into the event form at `output`.
fn manifest_sign(key_file: &str, input: &str, output: &str) {
    let secret = std::fs::read_to_string(key_file).unwrap_or_else(|e| {
        eprintln!("error: {key_file}: {e}");
        std::process::exit(1)
    });
    let keys = nostr::key::Keys::parse(secret.trim()).unwrap_or_else(|e| {
        eprintln!("error: {key_file}: {e}");
        std::process::exit(1)
    });
    let doc = std::fs::read_to_string(input).unwrap_or_else(|e| {
        eprintln!("error: {input}: {e}");
        std::process::exit(1)
    });
    let manifest = messenger_runtime::Manifest::parse_content(&doc).unwrap_or_else(die);
    let signed = manifest.sign(&keys).unwrap_or_else(die);
    std::fs::write(output, signed + "\n").unwrap_or_else(|e| {
        eprintln!("error: {output}: {e}");
        std::process::exit(1)
    });
    println!("signed manifest #{} by {} → {output}", manifest.serial, keys.public_key().to_hex());
}

fn write_private(path: &str, content: &str) {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(path).unwrap_or_else(|e| {
        eprintln!("error: {path}: {e}");
        std::process::exit(1)
    });
    f.write_all(content.as_bytes()).unwrap_or_else(|e| {
        eprintln!("error: {path}: {e}");
        std::process::exit(1)
    });
}

fn print_net(status: &messenger_runtime::net::NetStatus) {
    println!("mode      {}", status.mode.as_str());
    println!("way       {}", if status.active { "through a bridge" } else { "directly" });
    if let Some(bridge) = &status.bridge {
        println!("bridge    {bridge}");
    }
    println!("bridges   {} known, {} added here", status.bridges, status.private.len());
    for bridge in &status.private {
        println!("          {}#{}", bridge.addr, bridge.id);
    }
    if status.offer {
        println!("offer     the direct way is restricted; a bridge would help");
    }
    if !status.available {
        println!("(own servers are chosen: bridges carry only the project's servers)");
    }
}

fn die<T>(e: messenger_core::MessengerError) -> T {
    eprintln!("error: {e}");
    std::process::exit(1)
}

/// Accept the short `#abcdef12` form printed by `history`.
async fn full_id(rt: &MessengerRuntime, short: &str) -> String {
    let short = short.trim_start_matches('#');
    if short.len() == 64 {
        return short.to_string();
    }
    for c in rt.dm().list_chats(true).await.unwrap_or_else(die) {
        for m in rt.dm().messages(&c.id, None, 500).await.unwrap_or_else(die) {
            if m.id.starts_with(short) {
                return m.id;
            }
        }
    }
    eprintln!("error: no message starts with {short}");
    std::process::exit(1)
}

/// Publishing happens in the background; a command line tool must not
/// exit before its events left. Waits until the outbox is empty.
async fn flush(rt: &MessengerRuntime) {
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if rt.outbox().pending().await.unwrap_or(0) == 0 {
            return;
        }
    }
    eprintln!("warning: some events are still queued");
}

fn print_push(s: &messenger_runtime::push::PushStatus) {
    println!("state    {}", s.state);
    println!("server   {}{}", s.server.as_deref().unwrap_or("-"), if s.server_custom { " (named by hand)" } else { "" });
    println!("tell me  dm={} groups={}", s.dm, s.groups);
    if let Some(at) = s.expires_at {
        println!("expires  {at}");
    }
    if let Some(e) = &s.error {
        println!("error    {e}");
    }
    for r in &s.relays {
        println!(
            "relay    {:<12} {}{}",
            format!("{:?}", r.status).to_lowercase(),
            r.url,
            r.detail.as_deref().map(|d| format!("  ({d})")).unwrap_or_default()
        );
    }
}

fn take_switch(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

/// Accept the first characters of a group id.
async fn group_id(rt: &MessengerRuntime, short: &str) -> String {
    for g in messenger_store::groups::list(rt.store()).await.unwrap_or_else(die) {
        if g.id.starts_with(short) {
            return g.id;
        }
    }
    eprintln!("error: no group starts with {short}");
    std::process::exit(1)
}

/// Stay online long enough to hear what happened meanwhile: a command
/// that changes a group should know the group as it is now.
async fn settle(rt: &MessengerRuntime, secs: u64) {
    wait_connect(rt).await;
    tokio::time::sleep(Duration::from_secs(secs)).await;
}
