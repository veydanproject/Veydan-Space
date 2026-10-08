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
//! messenger-cli [--data-dir DIR] read <npub|hex> [SECONDS]
//! messenger-cli [--data-dir DIR] privacy [on|off|presence-on|presence-off]
//! messenger-cli [--data-dir DIR] presence [SECONDS] | presence-on | presence-rotate
//! messenger-cli [--data-dir DIR] edit <message-id> <text…>
//! messenger-cli [--data-dir DIR] delete <message-id>
//! messenger-cli [--data-dir DIR] react <message-id> <emoji>
//! messenger-cli [--data-dir DIR] emoji-top [N]
//! messenger-cli [--data-dir DIR] phone [<number>|none [--share]] | contact-phone <npub|hex>
//! messenger-cli [--data-dir DIR] card-send <npub|hex|group:ID> [me|<npub|hex>] [--phone]
//! messenger-cli [--data-dir DIR] cards <npub|hex|group:ID> [SECONDS] | card-accept <message-id>
//! messenger-cli [--data-dir DIR] relation <npub|hex>
//! messenger-cli [--data-dir DIR] request|accept|decline|block|unblock|remove <npub|hex>
//! messenger-cli [--data-dir DIR] push-on <token> [--server URL]
//! messenger-cli [--data-dir DIR] push-status | push-test | push-off
//! messenger-cli [--data-dir DIR] servers [veydan|own|refresh]
//! messenger-cli [--data-dir DIR] net [off|on|auto|check|add <address:port#id>|remove <id>]
//! messenger-cli [--data-dir DIR] call <npub|hex> [--video] [--relay-only] [--node <address:port#id>[,…]] [--audio-in tone[:HZ]|<file.wav>] [--audio-out <file.wav>] [--secs N]
//!   (--video: a video call; this side sends a moving test pattern of 640×360 at 30 fps whenever its video is on, and
//!    prints the size and the rate of what the far end sends; `video on`, `video off` and `camera` on stdin as well)
//! messenger-cli [--data-dir DIR] call-answer [--wait N] [the flags of call]   (one incoming call: answered, held until it ends)
//! messenger-cli [--data-dir DIR] call-wait [the flags of call]                (every incoming call, until Ctrl-C)
//! messenger-cli [--data-dir DIR] gcall <group:ID> [the flags of call]          (start a call in the group, stay in its room)
//! messenger-cli [--data-dir DIR] gcall-join <group:ID> [--wait N] [the flags of call]   (join the call that is on in the group)
//!   (--node <ref>: my own node; the room of another node is joined through it — the cascade, services/call/spec/cascade.md —
//!    and the lines say `via node B → home A`; a move of the room to another node is printed with the time out of it)
//! messenger-cli [--data-dir DIR] gcall-wait <group:ID> [the flags of call]     (join every call of the group, until Ctrl-C)
//! messenger-cli [--data-dir DIR] group-node <group:ID> <address:port#id|none> [--key K]   (pin a call node to the group)
//! messenger-cli manifest-keygen <secret-file>
//! messenger-cli manifest-sign --key-file <secret-file> <doc.json> <signed.json>
//! ```
//!
//! A call lives in the process that holds it: `call` and `call-answer`
//! stay in the call and read `end`, `mute`, `unmute` and `state` from
//! stdin until it is over; `gcall` and `gcall-join` stay in the room and
//! read `leave`, `mute`, `unmute`, `video on|off`, `layer <seat> <rid>`
//! and `state`, print the seats of the room as they change (who, verified,
//! speaking, the m-lines of their sound and video), the way to the node,
//! and at the end what came from every seat: the tones in its sound, the
//! frames of its video. The sound goes through the engine's pushed
//! path, 48 kHz mono: a tone or a WAV file in, a WAV file out, and the
//! received tail is measured for the tones of the other side (440 and
//! 660 Hz, the ones the live checks send) so that a run without a sound
//! card still says whether the sound came through. `gcall-wait` joins
//! the calls of the group one after another: never again the call it
//! left (the others may stay in it) or one it failed to join three
//! times, only the next one announced.
//!
//! Secrets live in `<data-dir>/secrets.json` in plaintext: development only.

use messenger_core::MessengerConfig;
use messenger_runtime::{MessengerRuntime, Paused};
use messenger_testkit::FileSecretStore;
use messenger_rtc::{has_test_square, test_pattern, AudioMode, AudioProcessing, AudioTap, RtcEngine, VideoFrame, VideoSource, VideoTap, FRAME_SAMPLES, SAMPLE_RATE};
use messenger_runtime::calls::VideoInput;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "usage: messenger-cli [--data-dir DIR] <keygen [--password PW] | import <nsec|ncryptsec> <secret> [--password PW] \
         | whoami | relays | relay-add <url> [--key K] | send <to> <text…> | tail | notify-tail | sync [secs] | chats | history <peer> | read <peer> [secs] | privacy [on|off|presence-on|presence-off] | presence [secs] | presence-on | presence-rotate | shared <peer|group:id> [visual|files|links|voice] | edit <id> <text…> | delete <id> | react <id> <emoji> | emoji-top [n] | phone [<number>|none [--share]] | contact-phone <peer> | card-send <peer|group:id> [me|<key>] [--phone] | cards <peer|group:id> [secs] | card-accept <id> | relation <peer> | request|accept|decline|block|unblock|remove <peer> | push-on <token> [--server URL] | push-status | push-test | push-off | profile-set <name> | wrap <to|group:ID|stranger:ID> <text…> [--send] | notify-describe <event.json> [--type dm|group] [--group ID] | servers [veydan|own|refresh] | net [off|on|auto|check|add <bridge>|remove <id>] | send-file <to> <path> [caption…] [--batch ID] [--original] [--pause-after N] [--cancel-after N] | download <msg> [--pause-after N] [--cancel-after N] | transfers | resume <transfer> [--pause-after N] [--cancel-after N] | pause|cancel <transfer> | call <peer> [--video] [--relay-only] [--node <ref>[,…]] [--audio-in tone[:HZ]|<wav>] [--audio-out <wav>] [--secs N] | call-answer [--wait N] [--secs N] [the flags of call] | call-wait [the flags of call] | gcall <group:id> [the flags of call] | gcall-join <group:id> [--wait N] [the flags of call] | gcall-wait <group:id> [the flags of call] | group-node <group:id> <ref|none> [--key K] | manifest-keygen <file> | manifest-sign --key-file F <doc.json> <signed.json>>"
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

    // One command and out: what an earlier run left is taken up by `resume
    // <id>`, never behind the back of the command.
    let config = MessengerConfig::new(data_dir.clone()).without_resume();
    let secrets = Arc::new(FileSecretStore::open(data_dir.join("secrets.json")).await.unwrap_or_else(die));
    // A call from the command line pushes its sound into the engine and
    // keeps what comes out (no sound card involved); every other command
    // leaves the engine of the runtime alone.
    let mut call_engine = None;
    let rt = if cmd.starts_with("call") || cmd.starts_with("gcall") {
        let engine = Arc::new(RtcEngine::new(AudioMode::Pushed(AudioProcessing::NONE)).unwrap_or_else(die));
        call_engine = Some(engine.clone());
        MessengerRuntime::start_with_engine(config, secrets, engine).await.unwrap_or_else(die)
    } else {
        MessengerRuntime::start(config, secrets).await.unwrap_or_else(die)
    };

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
                    "{} {} {:<8} {:<4} {}{}{}{}",
                    m.created_at,
                    if m.direction == "out" { "->" } else { "<-" },
                    m.status,
                    ticks(&m),
                    if m.deleted { "(deleted)".to_string() } else { body_line(&m) },
                    if m.edited_at.is_some() && !m.deleted { " (edited)" } else { "" },
                    reactions(&m),
                    format_args!("  #{}", &m.id[..8.min(m.id.len())]),
                );
            }
        }
        "read" => {
            // Stay a little first: what came while away is read too.
            if args.is_empty() {
                usage();
            }
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3);
            let chat = rt.chat_open(&args[0]).await.unwrap_or_else(die);
            settle(&rt, secs).await;
            rt.chat_mark_read(&chat.id).await.unwrap_or_else(die);
            rt.flush_receipts().await.unwrap_or_else(die);
            flush(&rt).await;
            println!("read {}", chat.id);
        }
        "privacy" => {
            let mut s = rt.privacy_settings().await.unwrap_or_else(die);
            match args.first().map(String::as_str) {
                None => {}
                Some("on") => s.read_receipts = true,
                Some("off") => s.read_receipts = false,
                Some("presence-on") => s.presence = true,
                Some("presence-off") => s.presence = false,
                Some(_) => usage(),
            }
            if !args.is_empty() {
                s = rt.privacy_set(s).await.unwrap_or_else(die);
                // Turned off, the contacts are told the key is gone.
                flush(&rt).await;
            }
            let word = |b: bool| if b { "on" } else { "off" };
            println!("read_receipts={} presence={}", word(s.read_receipts), word(s.presence));
        }
        "presence" => {
            // Watch the contacts' keys for a while, then say who is online.
            let secs: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(5);
            wait_connect(&rt).await;
            rt.presence_tick().await.unwrap_or_else(die);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            print_presence(&rt).await;
        }
        "presence-on" => {
            // In sight until Ctrl-C: tell my key, beat, and print what comes.
            wait_connect(&rt).await;
            print_my_presence(&rt).await;
            rt.presence_foreground(true).await.unwrap_or_else(die);
            rt.presence_tick().await.unwrap_or_else(die);
            print_presence(&rt).await;
            eprintln!("beating — Ctrl-C to stop");
            let mut rx = rt.ui_events();
            let mut lease = tokio::time::interval(Duration::from_secs(45));
            loop {
                tokio::select! {
                    _ = lease.tick() => rt.presence_foreground(true).await.unwrap_or_else(die),
                    ev = rx.recv() => match ev {
                        Ok(e) if e.name.starts_with("presence.") => println!("{} {}", e.name, e.payload),
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                        Err(_) => break,
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
            rt.presence_foreground(false).await.unwrap_or_else(die);
        }
        "presence-rotate" => {
            wait_connect(&rt).await;
            let epoch = rt.presence_rotate().await.unwrap_or_else(die);
            rt.presence_tick().await.unwrap_or_else(die);
            flush(&rt).await;
            println!("epoch {epoch}");
            print_my_presence(&rt).await;
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
        "react" => {
            // A second time takes it back.
            if args.len() != 2 {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            wait_connect(&rt).await;
            let m = rt.dm_react(&id, &args[1]).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("{} #{}{}", if m.reactions.iter().any(|r| r.mine && r.emoji == args[1]) { "put" } else { "taken back" }, &m.id[..8], reactions(&m));
        }
        "phone" => {
            // `phone`: shows mine; `phone <number|none> [--share]` sets it
            // and tells my other devices.
            let share = take_switch(&mut args, "--share");
            if let Some(number) = args.first() {
                wait_connect(&rt).await;
                let number = (number != "none").then_some(number.as_str());
                rt.own_private_set(number, share).await.unwrap_or_else(die);
                flush(&rt).await;
            }
            let p = rt.own_private_get().await.unwrap_or_else(die);
            println!("phone {}  share={}", p.phone.unwrap_or_else(|| "-".into()), p.share_phone);
        }
        "contact-phone" => {
            if args.is_empty() {
                usage();
            }
            let p = rt.contact_private_get(&args[0]).await.unwrap_or_else(die);
            println!("{}  phone {}", p.pubkey, p.phone.unwrap_or_else(|| "-".into()));
        }
        "card-send" => {
            // `card-send <peer|group:ID> [me|<npub|hex>] [--phone]`
            let phone = take_switch(&mut args, "--phone");
            if args.is_empty() {
                usage();
            }
            let to = chat_target(&rt, &args[0]).await;
            let whose = args.get(1).filter(|w| *w != "me").cloned();
            settle(&rt, 3).await;
            let m = rt.card_send(&to, whose.as_deref(), phone).await.unwrap_or_else(die);
            flush(&rt).await;
            println!("queued #{}  {}", &m.id[..8], m.card.as_ref().map(card_line).unwrap_or_default());
        }
        "cards" => {
            // `cards <peer|group:ID> [SECONDS]`: the cards of a chat.
            if args.is_empty() {
                usage();
            }
            let to = chat_target(&rt, &args[0]).await;
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            if secs > 0 {
                settle(&rt, secs).await;
            }
            let chat_id = match to.strip_prefix("group:") {
                Some(_) => to.clone(),
                None => rt.chat_open(&to).await.unwrap_or_else(die).id,
            };
            for m in rt.dm().messages(&chat_id, None, 500).await.unwrap_or_else(die) {
                if let Some(c) = &m.card {
                    println!("{} {} #{}  {}", m.created_at, if m.direction == "out" { "->" } else { "<-" }, &m.id[..8], card_line(c));
                }
            }
        }
        "card-accept" => {
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            wait_connect(&rt).await;
            let m = rt.card_accept(&id).await.unwrap_or_else(die);
            flush(&rt).await;
            let c = m.card.unwrap_or_else(|| die(messenger_core::MessengerError::Invalid("card_unknown".into())));
            let kept = rt.contact_private_get(&c.pubkey).await.unwrap_or_else(die);
            println!("added {}  contact={}  phone kept {}", c.npub, c.is_contact, kept.phone.unwrap_or_else(|| "-".into()));
        }
        "emoji-top" => {
            let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(24);
            println!("{}", rt.emoji_top(n).await.unwrap_or_else(die).join(" "));
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
            // send-file <to> <path> [caption…] [--batch <id>] [--original] [--pause-after N] [--cancel-after N]
            if args.len() < 2 {
                usage();
            }
            // `--batch <id>`: files sent with the same id are one album.
            let batch = take_flag(&mut args, "--batch");
            // `--original`: a photo goes as it is, not made smaller.
            let original = take_switch(&mut args, "--original");
            let mut steer = Steer::take(&mut args);
            let to = args.remove(0);
            let path = PathBuf::from(args.remove(0));
            let caption = if args.is_empty() { None } else { Some(args.join(" ")) };
            wait_connect(&rt).await;
            let started = std::time::Instant::now();
            // The preparing stage comes before any run: only events tell it.
            let mut events = rt.ui_events();
            let ph = rt
                .dm_send_file(&to, &path, caption.as_deref(), batch.as_deref(), original)
                .await
                .unwrap_or_else(die);
            let tid = ph.media.as_ref().and_then(|m| m["transfer_id"].as_str().map(String::from)).unwrap_or_default();
            eprintln!("  transfer {tid}");
            let mut last = String::new();
            let mut sent_as = String::new();
            loop {
                steer.wait().await;
                while let Ok(ev) = events.try_recv() {
                    let p = &ev.payload;
                    let ours = ev.name == messenger_runtime::media::UI_EVENT_TRANSFER && p["transfer_id"] == tid.as_str();
                    if ours && p["stage"] == "preparing" {
                        let (name, size) = (p["file_name"].as_str().unwrap_or(""), p["total_bytes"].as_u64().unwrap_or(0));
                        eprintln!("  {:<13} {:<11} {name} {}", "queued", "preparing", human(size));
                    }
                }
                let Some(t) = rt.media().transfer(&tid).await.unwrap_or_else(die) else { break };
                let file = format!("{} {}", t.file_name, human(t.size));
                if file != sent_as {
                    if !sent_as.is_empty() {
                        eprintln!("  sent as {file}");
                    }
                    sent_as = file;
                }
                let line = transfer_line(&t);
                if line != last {
                    eprintln!("  {line}");
                    last = line;
                }
                steer.check(&rt, &t).await;
                if matches!(t.status.as_str(), "done" | "failed" | "cancelled" | "paused") {
                    flush(&rt).await;
                    removals_done(&rt).await;
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
            // download <message id> [--pause-after N] [--cancel-after N]
            let mut steer = Steer::take(&mut args);
            if args.is_empty() {
                usage();
            }
            let id = full_id(&rt, &args[0]).await;
            let started = std::time::Instant::now();
            // The download runs here; its view is read and steered beside
            // it, and it goes on meanwhile: a cancel waits for its end.
            let download = rt.media_download(&id, true);
            tokio::pin!(download);
            let mut last = String::new();
            let result = loop {
                let look = async {
                    steer.wait().await;
                    if let Some(t) = rt.media_transfer(&id).await.unwrap_or_else(die) {
                        let line = transfer_line(&t);
                        if line != last {
                            eprintln!("  {} {line}", t.id);
                            last = line;
                        }
                        steer.check(&rt, &t).await;
                    }
                };
                tokio::select! {
                    result = &mut download => break result,
                    _ = look => {}
                }
            };
            removals_done(&rt).await;
            match result.unwrap_or_else(die) {
                Some(p) => println!("{} ({:.1}s)", p.display(), started.elapsed().as_secs_f32()),
                None => println!("not downloaded"),
            }
        }
        "transfers" => {
            for t in rt.media().active_transfers().await.unwrap_or_else(die) {
                println!("{} {:<4} {} {}", t.id, t.direction, transfer_line(&t), t.file_name);
            }
        }
        "pause" => {
            // pause <transfer id>. A transfer runs in the process that
            // started it (send-file, download, resume, or the app); it is
            // asked there and pauses itself. One whose run is gone is
            // paused here.
            if args.is_empty() {
                usage();
            }
            let paused = rt.media().pause(&args[0]).await.unwrap_or_else(die);
            if paused == Paused::Nothing {
                println!("{}: not running anywhere", args[0]);
            } else {
                if paused == Paused::ByRun {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                match rt.media().transfer(&args[0]).await.unwrap_or_else(die) {
                    Some(t) => println!("{} {}", t.id, transfer_line(&t)),
                    None => println!("unknown transfer"),
                }
            }
        }
        "cancel" => {
            // cancel <transfer id>: one that runs, here or in another
            // process (the run there cancels itself), or one that does not.
            if args.is_empty() {
                usage();
            }
            rt.media_cancel(&args[0]).await.unwrap_or_else(die);
            removals_done(&rt).await;
            match rt.media().transfer(&args[0]).await.unwrap_or_else(die) {
                Some(t) => println!("{} {}", t.id, transfer_line(&t)),
                None => println!("unknown transfer"),
            }
        }
        "resume" => {
            // resume <transfer id> [--pause-after N] [--cancel-after N]
            let mut steer = Steer::take(&mut args);
            if args.is_empty() {
                usage();
            }
            wait_connect(&rt).await;
            let started = std::time::Instant::now();
            rt.media_resume(&args[0]).await.unwrap_or_else(die);
            let mut last = String::new();
            loop {
                steer.wait().await;
                let Some(t) = rt.media().transfer(&args[0]).await.unwrap_or_else(die) else { break };
                let line = transfer_line(&t);
                if line != last {
                    eprintln!("  {line}");
                    last = line;
                }
                steer.check(&rt, &t).await;
                if matches!(t.status.as_str(), "done" | "failed" | "cancelled" | "paused") {
                    flush(&rt).await;
                    removals_done(&rt).await;
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
            for k in ["kind", "mime", "name", "size", "duration_ms", "dim"] {
                println!("{k}: {}", m.get(k).map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
            }
            println!("waveform: {} values", m.get("waveform").and_then(|w| w.as_array()).map(|a| a.len()).unwrap_or(0));
            // The preview the message carries: about how many bytes of JPEG.
            let thumb = m.get("thumb").and_then(|v| v.as_str()).map(|t| t.len() / 4 * 3);
            println!("thumb: {}", thumb.map(|n| format!("~{n} bytes")).unwrap_or_else(|| "-".into()));
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
            // `profile-set <name>`: publishes this identity's kind 0 (the
            // picture is the avatar's own, set by the app).
            if args.is_empty() {
                usage();
            }
            wait_connect(&rt).await;
            let input = messenger_runtime::ProfileInput {
                name: Some(args[0].clone()),
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
                    if m.deleted { "(deleted)".to_string() } else if m.content_type == "system" { format!("* {}", m.text.clone().unwrap_or_default()) } else { body_line(&m) },
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
                        messenger_runtime::GroupOp::EditSettings { name: Some(args[1..].join(" ")), about: None, picture: None, history_for_new: None, call_node: None, call_node_key: None },
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
        "call" | "call-answer" | "call-wait" => {
            let engine = call_engine.take().expect("the engine of the call commands");
            let plan = CallPlan::take(&mut args, &cmd);
            match cmd.as_str() {
                "call" => {
                    let Some(peer) = args.first().cloned() else { usage() };
                    call_out(&rt, &engine, &peer, &plan).await;
                }
                "call-answer" => call_in(&rt, &engine, &plan, true).await,
                _ => call_in(&rt, &engine, &plan, false).await,
            }
        }
        "gcall" | "gcall-join" | "gcall-wait" => {
            let engine = call_engine.take().expect("the engine of the call commands");
            let plan = CallPlan::take(&mut args, &cmd);
            let Some(group) = args.first() else { usage() };
            let group = group_id(&rt, group.trim_start_matches("group:")).await;
            match cmd.as_str() {
                "gcall" => gcall_start(&rt, &engine, &group, &plan).await,
                "gcall-join" => gcall_wait(&rt, &engine, &group, &plan, true).await,
                _ => gcall_wait(&rt, &engine, &group, &plan, false).await,
            }
        }
        "group-node" => {
            let key = take_flag(&mut args, "--key");
            if args.len() < 2 {
                usage();
            }
            let id = group_id(&rt, args[0].trim_start_matches("group:")).await;
            let node = if args[1] == "none" { String::new() } else { args[1].clone() };
            settle(&rt, 3).await;
            let op = messenger_runtime::GroupOp::EditSettings {
                name: None,
                about: None,
                picture: None,
                history_for_new: None,
                call_node: Some(node),
                call_node_key: Some(key.unwrap_or_default()),
            };
            rt.group_act(&id, op).await.unwrap_or_else(die);
            flush(&rt).await;
            match rt.groups().call_node_of(&id).await.unwrap_or_else(die) {
                Some((node, key)) => println!("call node {node}{}", if key.is_some() { " (with a key)" } else { "" }),
                None => println!("call node none"),
            }
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

/// Pause or cancel a transfer that runs in this process (a transfer runs
/// where it was started): after so many chunks (`--pause-after N`,
/// `--cancel-after N`) or on Ctrl-C, which pauses it for `resume <id>`; a
/// second Ctrl-C quits.
struct Steer {
    pause_after: Option<u32>,
    cancel_after: Option<u32>,
    interrupted: bool,
    asked: bool,
}

impl Steer {
    fn take(args: &mut Vec<String>) -> Self {
        let n = |v: Option<String>| v.map(|s| s.parse::<u32>().unwrap_or_else(|_| usage()));
        let pause_after = n(take_flag(args, "--pause-after"));
        let cancel_after = n(take_flag(args, "--cancel-after"));
        Self { pause_after, cancel_after, interrupted: false, asked: false }
    }

    /// A moment between two looks, cut short by Ctrl-C.
    async fn wait(&mut self) {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(300)) => {}
            _ = tokio::signal::ctrl_c() => {
                if self.interrupted {
                    std::process::exit(130);
                }
                self.interrupted = true;
            }
        }
    }

    /// Stop `t` once it is time.
    async fn check(&mut self, rt: &MessengerRuntime, t: &messenger_runtime::TransferView) {
        if self.asked || !matches!(t.status.as_str(), "queued" | "running" | "waiting_retry") {
            return;
        }
        let reached = |n: Option<u32>| n.is_some_and(|n| t.chunks_done >= n);
        if reached(self.cancel_after) {
            self.asked = true;
            eprintln!("  cancel at chunk {}/{}", t.chunks_done, t.chunks_total);
            rt.media_cancel(&t.id).await.unwrap_or_else(die);
        } else if self.interrupted || reached(self.pause_after) {
            self.asked = true;
            eprintln!("  pause at chunk {}/{}", t.chunks_done, t.chunks_total);
            rt.media_pause(&t.id).await.unwrap_or_else(die);
        }
    }
}

/// `1.5 MiB` and the like.
fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.2} GiB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MiB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

/// Status, stage, chunks, bytes, speed, time left and the next retry of a
/// transfer, on one line.
fn transfer_line(t: &messenger_runtime::TransferView) -> String {
    let mut line = format!(
        "{:<13} {:<11} chunk {}/{} {}/{}",
        t.status,
        format!("{:?}", t.stage).to_lowercase(),
        t.chunks_done,
        t.chunks_total,
        human(t.done_bytes),
        human(t.size)
    );
    if t.rate_bps > 0 {
        line.push_str(&format!(" {}/s", human(t.rate_bps)));
    }
    if let Some(eta) = t.eta_secs {
        line.push_str(&format!(" {eta}s left"));
    }
    if let Some(at) = t.retry_at_ms {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        line.push_str(&format!(" retry in {}s", (at - now).max(0) / 1000));
    }
    if let Some(reason) = &t.failure_reason {
        line.push_str(&format!(" {reason}"));
    }
    line
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

/// Wait while chunks of a cancelled upload leave the server: they go in
/// the background, and would stay there were the CLI to end first.
async fn removals_done(rt: &MessengerRuntime) {
    let removals = rt.media().removals_done();
    tokio::pin!(removals);
    if tokio::time::timeout(Duration::from_millis(200), &mut removals).await.is_ok() {
        return;
    }
    eprintln!("  removing its chunks from the server…");
    if tokio::time::timeout(Duration::from_secs(120), removals).await.is_err() {
        eprintln!("warning: some chunks may still be on the server");
    }
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

/// The marks of a message of mine, as the app draws them: sent, delivered,
/// read. Nothing for the peer's messages, nor for mine that did not leave.
fn ticks(m: &messenger_runtime::MessageView) -> &'static str {
    if m.direction != "out" || m.status != "sent" || m.content_type == "system" {
        ""
    } else if m.read_at.is_some() {
        "✓✓•"
    } else if m.delivered_at.is_some() {
        "✓✓"
    } else {
        "✓"
    }
}

/// What stands under a message: `  [👍2* ❤️1]`, a star on mine.
fn reactions(m: &messenger_runtime::MessageView) -> String {
    if m.reactions.is_empty() {
        return String::new();
    }
    let all: Vec<String> = m.reactions.iter().map(|r| format!("{}{}{}", r.emoji, r.count, if r.mine { "*" } else { "" })).collect();
    format!("  [{}]", all.join(" "))
}

/// Stay online long enough to hear what happened meanwhile: a command
/// that changes a group should know the group as it is now.
async fn settle(rt: &MessengerRuntime, secs: u64) {
    wait_connect(rt).await;
    tokio::time::sleep(Duration::from_secs(secs)).await;
}

/// Who of my contacts is online, and when the others were last seen.
async fn print_presence(rt: &MessengerRuntime) {
    let now = messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs();
    let list = rt.presence_list().await.unwrap_or_else(die);
    if list.is_empty() {
        println!("(nobody to show)");
    }
    for v in list {
        if now < v.online_until {
            println!("{}  online (until +{} s)", v.peer, v.online_until - now);
        } else {
            println!("{}  last seen {} s ago", v.peer, now - v.seen_at);
        }
    }
}

/// The key my beats are signed with now.
async fn print_my_presence(rt: &MessengerRuntime) {
    let keys = rt.identity().load_keys().await.unwrap_or_else(die);
    let (epoch, mine) = rt.presence().presence_keys(&keys).await.unwrap_or_else(die);
    println!("presence key {} (epoch {epoch})", mine.public_key().to_hex());
}

/// A chat as the commands name it: a person, or `group:<id prefix>`.
async fn chat_target(rt: &MessengerRuntime, name: &str) -> String {
    match name.strip_prefix("group:") {
        Some(short) => format!("group:{}", group_id(rt, short).await),
        None => name.to_string(),
    }
}

/// One line of a card: whose, its name, what it carries.
fn card_line(c: &messenger_runtime::CardView) -> String {
    format!(
        "card {}  \"{}\"  phone {}  avatar {}  socials {}{}{}",
        c.npub,
        c.label,
        c.phone.as_deref().unwrap_or("-"),
        c.avatar.as_ref().map_or("-".to_string(), |a| format!("{} bytes", a.len())),
        c.socials.len(),
        if c.is_me { "  (me)" } else { "" },
        if c.is_contact { "  (contact)" } else { "" },
    )
}

/// What a message shows: its text, its card, or its type.
fn body_line(m: &messenger_runtime::MessageView) -> String {
    match (&m.text, &m.card) {
        (Some(t), _) => t.clone(),
        (None, Some(c)) => card_line(c),
        (None, None) => format!("[{}]", m.content_type),
    }
}

// ─── Calls ───────────────────────────────────────────────────────────────────

/// The tones the live checks send: the caller the first, the called side
/// the second, a third participant of a room the third; the received
/// tail is measured for all of them, whichever side this is.
const TONES: [f64; 3] = [440.0, 660.0, 880.0];
/// How much of the end of the received sound is measured: past the ramp
/// of the jitter buffer.
const TAIL_SECS: usize = 2;

/// What a call from the command line sends and keeps.
struct CallPlan {
    video: bool,
    relay_only: bool,
    nodes: Vec<String>,
    source: AudioSource,
    out_path: Option<PathBuf>,
    /// Hang up this long after the call became active.
    secs: Option<u64>,
    /// `call-answer`: how long to wait for the call to come.
    wait: u64,
}

enum AudioSource {
    Tone(f64),
    /// 48 kHz mono, played once; silence after it.
    Wav(Vec<i16>),
}

impl CallPlan {
    fn take(args: &mut Vec<String>, cmd: &str) -> Self {
        let video = take_switch(args, "--video");
        let relay_only = take_switch(args, "--relay-only");
        let nodes = take_flag(args, "--node").map(|s| s.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
        let source = match take_flag(args, "--audio-in").as_deref() {
            // The caller sends the first tone, the answering side the second
            // (a third seat of a room says `--audio-in tone:880`).
            None | Some("tone") => AudioSource::Tone(if cmd == "call" || cmd == "gcall" { TONES[0] } else { TONES[1] }),
            Some(spec) if spec.starts_with("tone:") => AudioSource::Tone(spec[5..].parse().unwrap_or_else(|_| usage())),
            Some(path) => AudioSource::Wav(read_wav(path)),
        };
        let out_path = take_flag(args, "--audio-out").map(PathBuf::from);
        let secs = take_flag(args, "--secs").map(|s| s.parse().unwrap_or_else(|_| usage()));
        let wait = take_flag(args, "--wait").map(|s| s.parse().unwrap_or_else(|_| usage())).unwrap_or(90);
        Self { video, relay_only, nodes, source, out_path, secs, wait }
    }

    /// The policy and the nodes of the call, into the settings.
    async fn apply(&self, rt: &MessengerRuntime) {
        let policy = if self.relay_only { messenger_runtime::RelayPolicy::RelayOnly } else { messenger_runtime::RelayPolicy::Auto };
        rt.call_set_policy(policy).await.unwrap_or_else(die);
        if !self.nodes.is_empty() {
            let nodes = self.nodes.iter().map(|r| messenger_runtime::CallNodeInput { reference: r.clone(), key: None }).collect();
            rt.call_set_nodes(nodes).await.unwrap_or_else(die);
        }
        let st = rt.call_state().await.unwrap_or_else(die);
        println!(
            "{} policy {:?} nodes [{}]",
            stamp(),
            st.policy,
            st.nodes.iter().map(|n| format!("{} ({})", n.reference, n.class)).collect::<Vec<_>>().join(", ")
        );
    }
}

/// `call <peer>`: ring the peer, stay in the call until it ends.
async fn call_out(rt: &MessengerRuntime, engine: &RtcEngine, peer: &str, plan: &CallPlan) {
    wait_connect(rt).await;
    plan.apply(rt).await;
    let mut events = rt.ui_events();
    let taps = engine.audio_taps().expect("the taps of the engine, once");
    let media = if plan.video { messenger_runtime::CallMedia::Video } else { messenger_runtime::CallMedia::Audio };
    let issued = std::time::Instant::now();
    println!("{} call {peer} {media:?}", stamp());
    let view = rt.call_start(peer, media).await.unwrap_or_else(die);
    println!("{} started {} phase {:?} nodes {:?}", stamp(), view.call_id, view.phase, view.nodes);
    in_call(rt, &mut events, taps, plan, &view.call_id, issued).await;
    flush(rt).await;
}

/// `call-answer` (one call) and `call-wait` (every call): answer what rings.
async fn call_in(rt: &MessengerRuntime, engine: &RtcEngine, plan: &CallPlan, once: bool) {
    wait_connect(rt).await;
    plan.apply(rt).await;
    let mut events = rt.ui_events();
    let mut taps = engine.audio_taps().expect("the taps of the engine, once");
    let me = rt.identity().get().await.unwrap_or_else(die).map(|i| i.npub).unwrap_or_default();
    println!("{} waiting for a call as {me}{}", stamp(), if once { format!(" (up to {} s)", plan.wait) } else { " — Ctrl-C to stop".into() });
    loop {
        let deadline = tokio::time::sleep(Duration::from_secs(if once { plan.wait } else { u64::MAX / 4 }));
        tokio::pin!(deadline);
        let call_id = loop {
            tokio::select! {
                ev = events.recv() => match ev {
                    Ok(e) if e.name == messenger_runtime::UI_EVENT_CALL_INCOMING => {
                        let call = e.payload["call"].clone();
                        println!("{} call.incoming from {} {} media {}", stamp(), call["peer"].as_str().unwrap_or("?"), call["call_id"].as_str().unwrap_or("?"), call["media"]);
                        break call["call_id"].as_str().unwrap_or_default().to_string();
                    }
                    Ok(e) if e.name == messenger_runtime::UI_EVENT_CALL_ENDED => {
                        // Missed while away, or declined elsewhere: on record, not for us.
                        println!("{} call.ended {} ({})", stamp(), e.payload["call"]["call_id"].as_str().unwrap_or("?"), e.payload["outcome"]);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                    Err(_) => return,
                },
                _ = &mut deadline => {
                    println!("{} no call came in {} s", stamp(), plan.wait);
                    return;
                }
                _ = tokio::signal::ctrl_c() => return,
            }
        };
        let answered = std::time::Instant::now();
        match rt.call_accept(&call_id).await {
            Ok(view) => println!("{} answered {} phase {:?} nodes {:?}", stamp(), view.call_id, view.phase, view.nodes),
            Err(e) => {
                eprintln!("{} accept failed: {e}", stamp());
                continue;
            }
        }
        in_call(rt, &mut events, &mut taps, plan, &call_id, answered).await;
        flush(rt).await;
        if once {
            return;
        }
        println!("{} waiting for the next call — Ctrl-C to stop", stamp());
    }
}

/// The call from here to its end: the sound pumped and kept, the video
/// pushed while wanted and counted as it comes, the events printed,
/// stdin read for `end`, `mute`, `unmute`, `video on|off`, `camera`,
/// `state`; `--secs` hangs up after that long active; Ctrl-C hangs up.
async fn in_call(
    rt: &MessengerRuntime,
    events: &mut tokio::sync::broadcast::Receiver<messenger_core::traits::UiEvent>,
    mut taps: impl std::borrow::BorrowMut<tokio::sync::mpsc::UnboundedReceiver<AudioTap>>,
    plan: &CallPlan,
    call_id: &str,
    since: std::time::Instant,
) {
    let taps = taps.borrow_mut();
    let mut pump: Option<tokio::task::JoinHandle<()>> = None;
    // What came from the far end so far: read here once the call is over,
    // since the far end's stream does not end by itself.
    let got: Arc<std::sync::Mutex<Vec<i16>>> = Arc::default();
    let mut keep: Option<tokio::task::JoinHandle<()>> = None;
    let got_video: Arc<std::sync::Mutex<VideoGot>> = Arc::default();
    let mut video_tasks: Vec<tokio::task::JoinHandle<()>> = vec![];
    let mut active_at: Option<std::time::Instant> = None;
    let hang_up = tokio::time::sleep(Duration::from_secs(u64::MAX / 4));
    tokio::pin!(hang_up);
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = tokio::io::AsyncBufReadExt::lines(stdin);
    let mut stdin_open = true;
    let mut last_stats = String::new();
    let mut outcome = None;
    loop {
        tokio::select! {
            tap = taps.recv() => {
                let Some(tap) = tap else { continue };
                println!("{} audio: the engine took the sound of this side", stamp());
                pump = Some(pump_audio(tap.input, &plan.source));
                let VideoTap { source, wanted, remote } = tap.video;
                video_tasks.push(pump_video(source, wanted));
                video_tasks.push(watch_video(remote, got_video.clone()));
                let got = got.clone();
                keep = Some(tokio::spawn(async move {
                    let Ok(mut output) = tap.output.await else { return };
                    while let Some(frame) = output.next().await {
                        got.lock().unwrap().extend_from_slice(&frame);
                    }
                }));
            }
            ev = events.recv() => match ev {
                Ok(e) if e.name.starts_with("call.") => {
                    if e.payload["call"]["call_id"] != call_id && e.payload["call_id"] != call_id {
                        continue;
                    }
                    match e.name.as_str() {
                        messenger_runtime::UI_EVENT_CALL_STATE => {
                            let c = &e.payload["call"];
                            let size = |s: &serde_json::Value| if s.is_null() { "-".to_string() } else { format!("{}x{}", s["width"], s["height"]) };
                            println!(
                                "{} call.state {}{} via {} muted {} nodes {} video mine {}{} {} theirs {} {} (+{:.2} s)",
                                stamp(),
                                c["phase"].as_str().unwrap_or("?"),
                                c["reconnect_reason"].as_str().map(|r| format!(" ({r})")).unwrap_or_default(),
                                c["via"].as_str().unwrap_or("-"),
                                c["muted"],
                                c["nodes"],
                                c["video_local"],
                                if c["video_screen"] == true { " (screen)" } else { "" },
                                size(&c["video_local_size"]),
                                c["video_remote"],
                                size(&c["video_remote_size"]),
                                since.elapsed().as_secs_f32()
                            );
                            if c["phase"] == "active" && active_at.is_none() {
                                active_at = Some(std::time::Instant::now());
                                if let Some(secs) = plan.secs {
                                    hang_up.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(secs));
                                }
                            }
                        }
                        messenger_runtime::UI_EVENT_CALL_ENDED => {
                            outcome = Some((e.payload["outcome"].to_string(), e.payload["duration_secs"].to_string()));
                            break;
                        }
                        "call.stats" => {
                            let s = &e.payload["stats"];
                            let line = format!(
                                "rtt {} ms, sent {} B, received {} B, lost {}, jitter {} ms",
                                s["rtt_ms"], s["bytes_sent"], s["bytes_received"], s["packets_lost"], s["jitter_ms"]
                            );
                            if line != last_stats {
                                println!("{} call.stats {line}", stamp());
                                last_stats = line;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                Err(_) => break,
            },
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(line)) => match line.trim() {
                    "end" | "call-end" => { let _ = rt.call_end(call_id).await.map_err(|e| eprintln!("end: {e}")); }
                    "mute" => { let _ = rt.call_set_mute(true).await.map_err(|e| eprintln!("mute: {e}")); }
                    "unmute" => { let _ = rt.call_set_mute(false).await.map_err(|e| eprintln!("unmute: {e}")); }
                    "video on" => { let _ = rt.call_set_video(VideoInput::Camera { id: None }).await.map_err(|e| eprintln!("video on: {e}")); }
                    "video off" => { let _ = rt.call_set_video(VideoInput::Off).await.map_err(|e| eprintln!("video off: {e}")); }
                    "camera" => match rt.call_switch_camera(None).await {
                        Ok(v) => println!("{} camera {:?}", stamp(), v.camera),
                        Err(e) => eprintln!("camera: {e}"),
                    },
                    "state" => println!("{} {}", stamp(), serde_json::to_string(&rt.call_state().await.unwrap_or_else(die)).unwrap_or_default()),
                    // What the platform says on a change of the network: an
                    // ICE restart at once (the caller offers, the called side asks).
                    "restart" => {
                        println!("{} restart: the network changed", stamp());
                        rt.call_network_changed().await;
                    }
                    "" => {}
                    other => eprintln!("(unknown: {other}; end, mute, unmute, video on, video off, camera, state, restart)"),
                },
                // No stdin (a pipe that closed, /dev/null): the call goes on without it.
                _ => stdin_open = false,
            },
            _ = &mut hang_up => {
                hang_up.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(u64::MAX / 4));
                println!("{} hanging up after {} s", stamp(), plan.secs.unwrap_or(0));
                let _ = rt.call_end(call_id).await.map_err(|e| eprintln!("end: {e}"));
            }
            _ = tokio::signal::ctrl_c() => {
                println!("{} Ctrl-C: hanging up", stamp());
                let _ = rt.call_end(call_id).await.map_err(|e| eprintln!("end: {e}"));
            }
        }
    }
    if let Some(p) = pump {
        p.abort();
    }
    for t in video_tasks {
        t.abort();
    }
    let (outcome, duration) = outcome.unwrap_or_else(|| ("\"?\"".into(), "null".into()));
    println!("{} call.ended outcome {outcome} duration {duration} s{}", stamp(), active_at.map(|a| format!(" (active for {:.1} s here)", a.elapsed().as_secs_f32())).unwrap_or_default());
    if let Some(keep) = keep {
        // A moment for the last frames, then the stream is let go.
        tokio::time::sleep(Duration::from_millis(500)).await;
        keep.abort();
        let got = std::mem::take(&mut *got.lock().unwrap());
        report_received(&got, plan.out_path.as_deref());
    }
    let v = got_video.lock().unwrap();
    println!(
        "{} video received {} frames; sizes (width x height, rotation) {:?}; best second {} fps; the test pattern seen: {}",
        stamp(),
        v.frames,
        v.sizes,
        v.peak_fps,
        v.pattern
    );
}

/// What came of the far end's video.
#[derive(Default)]
struct VideoGot {
    frames: u64,
    /// Every size seen, as it changed: width, height, rotation.
    sizes: Vec<(u32, u32, u16)>,
    peak_fps: u32,
    /// Some frame showed the bright square of the test pattern.
    pattern: bool,
}

/// Pushes the test pattern of 640×360 into `source` at 30 frames a second
/// while `wanted` says my video is on (the core turned it on: a video
/// call, or `video on`), and nothing while it is off.
fn pump_video(source: VideoSource, mut wanted: tokio::sync::watch::Receiver<bool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut seq = 0u32;
        loop {
            if !*wanted.borrow() {
                if wanted.changed().await.is_err() {
                    return;
                }
                continue;
            }
            println!("{} video: pushing the test pattern 640x360 at 30 fps", stamp());
            let mut tick = tokio::time::interval(Duration::from_micros(1_000_000 / 30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let pushed_from = seq;
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let mut frame = test_pattern(640, 360, seq);
                        frame.timestamp_us = started.elapsed().as_micros() as i64;
                        source.push(Arc::new(frame));
                        seq += 1;
                    }
                    changed = wanted.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        if !*wanted.borrow() {
                            break;
                        }
                    }
                }
            }
            println!("{} video: off after {} frames pushed", stamp(), seq - pushed_from);
        }
    })
}

/// Counts the far end's frames by the second and says so, with the size
/// and the rotation whenever they change.
fn watch_video(remote: tokio::sync::broadcast::Receiver<Arc<VideoFrame>>, got: Arc<std::sync::Mutex<VideoGot>>) -> tokio::task::JoinHandle<()> {
    watch_video_of("", remote, got)
}

/// [`watch_video`] of one video among several: `label` names it in what
/// is printed (the seat of a room and the m-line of its video).
fn watch_video_of(label: &str, mut remote: tokio::sync::broadcast::Receiver<Arc<VideoFrame>>, got: Arc<std::sync::Mutex<VideoGot>>) -> tokio::task::JoinHandle<()> {
    let label = if label.is_empty() { String::new() } else { format!("{label}: ") };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut in_second = 0u32;
        let mut size: Option<(u32, u32, u16)> = None;
        loop {
            tokio::select! {
                frame = remote.recv() => match frame {
                    Ok(f) => {
                        in_second += 1;
                        let s = (f.width, f.height, f.rotation);
                        let mut g = got.lock().unwrap();
                        g.frames += 1;
                        let pattern = has_test_square(&f);
                        g.pattern |= pattern;
                        if size != Some(s) {
                            size = Some(s);
                            g.sizes.push(s);
                            println!("{} video: {label}frames of {}x{} rotation {} ({})", stamp(), s.0, s.1, s.2, if pattern { "the test pattern" } else { "not the test pattern" });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = tick.tick() => {
                    if in_second > 0 {
                        let mut g = got.lock().unwrap();
                        g.peak_fps = g.peak_fps.max(in_second);
                        if let Some((w, h, _)) = size {
                            println!("{} video: {label}{in_second} fps {w}x{h}", stamp());
                        }
                    }
                    in_second = 0;
                }
            }
        }
    })
}

/// Pushes the source into `input` every 10 ms: the tone forever, the
/// file once and silence after it.
fn pump_audio(input: messenger_rtc::AudioInput, source: &AudioSource) -> tokio::task::JoinHandle<()> {
    enum Feed {
        Tone { hz: f64, phase: f64 },
        Wav { samples: Vec<i16>, at: usize },
    }
    let mut feed = match source {
        AudioSource::Tone(hz) => Feed::Tone { hz: *hz, phase: 0.0 },
        AudioSource::Wav(samples) => Feed::Wav { samples: samples.clone(), at: 0 },
    };
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        let mut frame = vec![0i16; FRAME_SAMPLES];
        loop {
            tick.tick().await;
            match &mut feed {
                Feed::Tone { hz, phase } => {
                    let step = 2.0 * std::f64::consts::PI * *hz / SAMPLE_RATE as f64;
                    for s in frame.iter_mut() {
                        *s = (phase.sin() * 8000.0) as i16;
                        *phase = (*phase + step) % (2.0 * std::f64::consts::PI);
                    }
                }
                Feed::Wav { samples, at } => {
                    for s in frame.iter_mut() {
                        *s = samples.get(*at).copied().unwrap_or(0);
                        *at += 1;
                    }
                }
            }
            if let Err(e) = input.push(&mut frame).await {
                eprintln!("audio push: {e}");
                return;
            }
        }
    })
}

/// What came from the far end: how much, how loud its tail was and how
/// much of the tail is each of the tones; the whole of it to a file.
fn report_received(got: &[i16], out: Option<&std::path::Path>) {
    report_received_of("", got, out);
}

/// [`report_received`] of one sound among several (`label`: whose).
fn report_received_of(label: &str, got: &[i16], out: Option<&std::path::Path>) {
    let secs = got.len() as f64 / SAMPLE_RATE as f64;
    let tail = &got[got.len().saturating_sub(TAIL_SECS * SAMPLE_RATE as usize)..];
    let rms = if tail.is_empty() { 0.0 } else { (tail.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt() };
    let tones: Vec<String> = TONES.iter().map(|hz| format!("{hz} Hz {:.3}", tone_ratio(tail, *hz))).collect();
    let label = if label.is_empty() { String::new() } else { format!("{label}: ") };
    println!("{} audio received {label}{secs:.1} s; tail rms {rms:.0}; tone {}", stamp(), tones.join(", "));
    if let Some(path) = out {
        match write_wav(path, got) {
            Ok(()) => println!("{} audio written to {}", stamp(), path.display()),
            Err(e) => eprintln!("audio: {}: {e}", path.display()),
        }
    }
}

/// Goertzel power of `hz` in `samples` against the total power: 1.0 a
/// pure tone, 0.0 silence or noise.
fn tone_ratio(samples: &[i16], hz: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f64;
    let k = (0.5 + n * hz / SAMPLE_RATE as f64).floor();
    let w = 2.0 * std::f64::consts::PI * k / n;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2, mut total) = (0.0f64, 0.0f64, 0.0f64);
    for &x in samples {
        let x = x as f64;
        total += x * x;
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    if total > 0.0 { (power / (total * n / 2.0)).min(1.0) } else { 0.0 }
}

/// 16-bit PCM WAV, any rate and channel count, as 48 kHz mono.
fn read_wav(path: &str) -> Vec<i16> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("error: {path}: {e}");
        std::process::exit(1)
    });
    let bad = |why: &str| -> ! {
        eprintln!("error: {path}: {why}");
        std::process::exit(1)
    };
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bad("not a WAV file");
    }
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut data: &[u8] = &[];
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                if u16::from_le_bytes([body[0], body[1]]) != 1 {
                    bad("not PCM");
                }
                channels = u16::from_le_bytes([body[2], body[3]]);
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes([body[14], body[15]]);
            }
            b"data" => data = body,
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    if bits != 16 || channels == 0 || rate == 0 || data.is_empty() {
        bad("needs 16-bit PCM with a fmt and a data chunk");
    }
    let frames: Vec<i16> = data
        .chunks_exact(2 * channels as usize)
        .map(|f| {
            let sum: i32 = f.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as i32).sum();
            (sum / channels as i32) as i16
        })
        .collect();
    if rate == SAMPLE_RATE {
        return frames;
    }
    // Linear resampling: good enough for a check of the way, not for music.
    let out_len = (frames.len() as u64 * SAMPLE_RATE as u64 / rate as u64) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * rate as f64 / SAMPLE_RATE as f64;
            let (a, t) = (pos.floor() as usize, pos.fract());
            let x0 = frames.get(a).copied().unwrap_or(0) as f64;
            let x1 = frames.get(a + 1).copied().unwrap_or(x0 as i16) as f64;
            (x0 + (x1 - x0) * t) as i16
        })
        .collect()
}

/// 16-bit PCM WAV, 48 kHz mono.
fn write_wav(path: &std::path::Path, samples: &[i16]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out)
}

// ─── Group calls ─────────────────────────────────────────────────────────────

/// `gcall <group>`: start a call in the group, stay in its room until I
/// leave or it ends.
async fn gcall_start(rt: &MessengerRuntime, engine: &RtcEngine, group: &str, plan: &CallPlan) {
    wait_connect(rt).await;
    plan.apply(rt).await;
    let mut events = rt.ui_events();
    let mut taps = engine.audio_taps().expect("the taps of the engine, once");
    let media = if plan.video { messenger_runtime::CallMedia::Video } else { messenger_runtime::CallMedia::Audio };
    let issued = std::time::Instant::now();
    println!("{} gcall group:{} {media:?}", stamp(), &group[..12]);
    let view = rt.group_call_start(group, media).await.unwrap_or_else(die);
    println!("{} started {} phase {:?} {} seat {:?}", stamp(), view.call_id, view.phase, where_of(&view.node, &view.home), view.participant);
    in_room(rt, &mut events, &mut taps, plan, &view.call_id, issued).await;
    flush(rt).await;
}

/// How many times `gcall-wait` tries to join one call before it gives
/// that call up and waits for the next one.
const JOIN_TRIES: u32 = 3;

/// The call `gcall-wait` is done with: the one it just left (the others
/// may stay in it, so the group still announces it with `joined: false`)
/// or the one whose join failed `JOIN_TRIES` times (a stable reason).
/// That call is not joined again; the next `group_call.started` with a
/// new id is.
#[derive(Default)]
struct DoneWith {
    call_id: Option<String>,
    /// The call whose joins failed so far, and how many times.
    failing: Option<(String, u32)>,
}

impl DoneWith {
    /// Whether the announced `call_id` is to be waited past.
    fn skips(&self, call_id: &str) -> bool {
        self.call_id.as_deref() == Some(call_id)
    }

    /// Left `call_id` (or it ended): not again.
    fn left(&mut self, call_id: &str) {
        self.call_id = Some(call_id.to_string());
        self.failing = None;
    }

    /// A join of `call_id` failed: `true` to try it again, `false` once
    /// it failed `JOIN_TRIES` times (then it is skipped like a left one).
    fn failed(&mut self, call_id: &str) -> bool {
        let tries = match self.failing.take() {
            Some((id, n)) if id == call_id => n + 1,
            _ => 1,
        };
        if tries >= JOIN_TRIES {
            self.left(call_id);
            return false;
        }
        self.failing = Some((call_id.to_string(), tries));
        true
    }
}

/// `gcall-join` (one call) and `gcall-wait` (every call of the group):
/// join the call announced in the group, waiting for the announcement
/// when there is none yet.
async fn gcall_wait(rt: &MessengerRuntime, engine: &RtcEngine, group: &str, plan: &CallPlan, once: bool) {
    wait_connect(rt).await;
    plan.apply(rt).await;
    let mut events = rt.ui_events();
    let mut taps = engine.audio_taps().expect("the taps of the engine, once");
    let me = rt.identity().get().await.unwrap_or_else(die).map(|i| i.npub).unwrap_or_default();
    println!(
        "{} waiting for a call in group:{} as {me}{}",
        stamp(),
        &group[..12],
        if once { format!(" (up to {} s)", plan.wait) } else { " — Ctrl-C to stop".into() }
    );
    let mut done = DoneWith::default();
    loop {
        let deadline = tokio::time::sleep(Duration::from_secs(if once { plan.wait } else { u64::MAX / 4 }));
        tokio::pin!(deadline);
        // Announced already (the notes came before this command), or
        // announced while we wait; never the call I left or gave up.
        let mut announced = rt.group_call_state(Some(group)).await.announced.filter(|a| !a.joined && !done.skips(&a.call_id));
        if announced.is_none() {
            announced = loop {
                tokio::select! {
                    ev = events.recv() => match ev {
                        Ok(e) if e.name == messenger_runtime::UI_EVENT_GROUP_CALL_STARTED => {
                            let call = &e.payload["call"];
                            if call["group_id"] != group || call["joined"] == true || call["call_id"].as_str().is_some_and(|id| done.skips(id)) {
                                continue;
                            }
                            println!(
                                "{} group_call.started {} by {} media {} participants {}",
                                stamp(),
                                call["call_id"].as_str().unwrap_or("?"),
                                call["started_by"].as_str().map(|s| &s[..12]).unwrap_or("?"),
                                call["media"],
                                call["participants"]
                            );
                            break serde_json::from_value::<messenger_runtime::GroupCallAnnounced>(call.clone()).ok();
                        }
                        Ok(e) if e.name == messenger_runtime::UI_EVENT_GROUP_CALL_ENDED => {
                            println!("{} group_call.ended {} ({})", stamp(), e.payload["call"]["call_id"].as_str().unwrap_or("?"), e.payload["outcome"]);
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                        Err(_) => return,
                    },
                    _ = &mut deadline => {
                        println!("{} no call came in {} s", stamp(), plan.wait);
                        return;
                    }
                    _ = tokio::signal::ctrl_c() => return,
                }
            };
        }
        let Some(announced) = announced else { continue };
        let joining = std::time::Instant::now();
        match rt.group_call_join(group).await {
            Ok(view) => println!("{} joined {} phase {:?} {} seat {:?}", stamp(), view.call_id, view.phase, where_of(&view.node, &view.home), view.participant),
            Err(e) => {
                eprintln!("{} join failed: {e}", stamp());
                if once {
                    return;
                }
                if done.failed(&announced.call_id) {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                } else {
                    println!("{} giving up {} after {JOIN_TRIES} tries; waiting for the next call — Ctrl-C to stop", stamp(), announced.call_id);
                }
                continue;
            }
        }
        in_room(rt, &mut events, &mut taps, plan, &announced.call_id, joining).await;
        flush(rt).await;
        if once {
            return;
        }
        // The call may go on without me: it is not mine to join again.
        done.left(&announced.call_id);
        println!("{} waiting for the next call — Ctrl-C to stop", stamp());
    }
}

#[cfg(test)]
mod tests {
    use super::{DoneWith, Way, JOIN_TRIES};

    /// The first state of `gcall` (phase `starting`, no node yet) prints
    /// no way: no `node ` line with an empty address for a grep to take
    /// as the node of the room. The way is printed once known, once per
    /// change, and a change of home is a move.
    #[test]
    fn the_way_is_printed_once_known_and_once_per_change() {
        let mut way = Way::default();
        assert_eq!(way.moved("", ""), None, "starting: no node yet, nothing printed");
        assert_eq!(way.here(), "", "nothing known to repeat");
        assert_eq!(way.moved("a:8443", "").as_deref(), Some("node a:8443"), "the node answered: the way, home not yet told");
        assert_eq!(way.moved("a:8443", "a:8443"), None, "the home named as my own node: the same line, not repeated");
        assert_eq!(way.moved("a:8443", "a:8443"), None, "the same state again: silent");
        assert_eq!(way.moved("b:8443", "a:8443").as_deref(), Some("via node b:8443 → home a:8443"), "through my own node: the cascade");
        assert_eq!(way.moved("b:8443", "c:8443").as_deref(), Some("room moved: home a:8443 → via node b:8443 → home c:8443"), "the home died: the move is named");
        assert_eq!(way.here(), "via node b:8443 → home c:8443");
        assert_eq!(way.moved("", "c:8443"), None, "a state without a node forgets nothing");
        assert_eq!(way.here(), "via node b:8443 → home c:8443", "the last known way stays for the `back in the room` line");
    }

    /// The call `gcall-wait` left is not joined again while the group
    /// still announces it; the next call (a new id) is.
    #[test]
    fn a_left_call_is_waited_past_until_a_new_one() {
        let mut done = DoneWith::default();
        assert!(!done.skips("call-1"), "nothing left yet: the first call is joined");
        done.left("call-1");
        assert!(done.skips("call-1"), "the call I left, still live for the others, is not rejoined");
        assert!(!done.skips("call-2"), "a new call of the group is joined");
        done.left("call-2");
        assert!(!done.skips("call-1"), "only the last one left is skipped: a call started anew is new");
        assert!(done.skips("call-2"));
    }

    /// A join that fails is retried a bounded number of times, then the
    /// call is given up like a left one: no retry of the same call forever.
    #[test]
    fn a_failing_join_is_retried_then_given_up() {
        let mut done = DoneWith::default();
        for n in 1..JOIN_TRIES {
            assert!(done.failed("call-1"), "try {n} of {JOIN_TRIES}: retried");
            assert!(!done.skips("call-1"), "still wanted while retried");
        }
        assert!(!done.failed("call-1"), "the last try: given up");
        assert!(done.skips("call-1"), "the given-up call is waited past");
        assert!(!done.skips("call-2"), "the next call is joined");
        // The failures of one call do not count against another.
        for _ in 1..JOIN_TRIES {
            assert!(done.failed("call-2"), "the new call gets its own tries");
        }
        assert!(!done.skips("call-2"), "still wanted: its own tries are not out");
        assert!(done.failed("call-3"), "a third call starts its count afresh");
        assert!(!done.skips("call-2") && !done.skips("call-3"));
    }
}

/// What came of the sound and the video of every seat: by the m-lines
/// of their tracks, with the seat the room's state named for each.
#[derive(Default)]
struct RoomGot {
    /// The sound by its m-line.
    audio: std::collections::BTreeMap<String, Vec<i16>>,
    /// The video by its m-line.
    video: std::collections::BTreeMap<String, Arc<std::sync::Mutex<VideoGot>>>,
    /// Whose m-line each is, as the state last said.
    seats: std::collections::BTreeMap<String, u32>,
}

/// In the room from here to my leaving or the end of the call: the sound
/// pumped, the sound of every seat kept by its m-line, the video pushed
/// while wanted and the video of every seat counted, the seats printed
/// as they change (who, verified, speaking, their m-lines), the way to
/// the node, stdin read for `leave`, `mute`, `unmute`, `video on|off`,
/// `layer <seat> <rid>`, `state`; `--secs` leaves after that long in the
/// room; Ctrl-C leaves.
async fn in_room(
    rt: &MessengerRuntime,
    events: &mut tokio::sync::broadcast::Receiver<messenger_core::traits::UiEvent>,
    taps: &mut tokio::sync::mpsc::UnboundedReceiver<AudioTap>,
    plan: &CallPlan,
    call_id: &str,
    since: std::time::Instant,
) {
    let mut pump: Option<tokio::task::JoinHandle<()>> = None;
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = vec![];
    let got: Arc<std::sync::Mutex<RoomGot>> = Arc::default();
    let mut room_audio: Option<tokio::sync::mpsc::UnboundedReceiver<(String, messenger_rtc::AudioOutput)>> = None;
    let mut via: Option<tokio::sync::watch::Receiver<Option<messenger_calls::PairKind>>> = None;
    let mut in_room_at: Option<std::time::Instant> = None;
    let hang_up = tokio::time::sleep(Duration::from_secs(u64::MAX / 4));
    tokio::pin!(hang_up);
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = tokio::io::AsyncBufReadExt::lines(stdin);
    let mut stdin_open = true;
    let mut last_line = String::new();
    let mut outcome = None;
    // Where I sit (the node I am connected to, the home of the room) as
    // the state last said, and since when the way to the room is lost.
    let mut way = Way::default();
    let mut lost_at: Option<std::time::Instant> = None;
    loop {
        tokio::select! {
            tap = taps.recv() => {
                let Some(tap) = tap else { continue };
                println!("{} audio: the engine took the sound of this side", stamp());
                pump = Some(pump_audio(tap.input, &plan.source));
                let VideoTap { source, wanted, .. } = tap.video;
                tasks.push(pump_video(source, wanted));
                room_audio = Some(tap.room_audio);
                via = Some(tap.via);
            }
            came = async { room_audio.as_mut().expect("checked").recv().await }, if room_audio.is_some() => {
                match came {
                    Some((mid, mut output)) => {
                        println!("{} audio: the sound on mid {mid} arrived", stamp());
                        let got = got.clone();
                        tasks.push(tokio::spawn(async move {
                            while let Some(frame) = output.next().await {
                                got.lock().unwrap().audio.entry(mid.clone()).or_default().extend_from_slice(&frame);
                            }
                        }));
                    }
                    None => room_audio = None,
                }
            }
            changed = async { via.as_mut().expect("checked").changed().await }, if via.is_some() => {
                match changed {
                    Ok(()) => {
                        let way = *via.as_ref().expect("checked").borrow();
                        println!("{} via {}", stamp(), way.map(|w| w.as_str()).unwrap_or("-"));
                    }
                    Err(_) => via = None,
                }
            }
            ev = events.recv() => match ev {
                Ok(e) if e.name.starts_with("group_call.") => {
                    if e.payload["call"]["call_id"] != call_id && e.payload["call_id"] != call_id {
                        continue;
                    }
                    match e.name.as_str() {
                        messenger_runtime::UI_EVENT_GROUP_CALL_STATE => {
                            let c = &e.payload["call"];
                            let seats: Vec<String> = c["participants"]
                                .as_array()
                                .map(|list| list.iter().map(seat_line).collect())
                                .unwrap_or_default();
                            let speaking: Vec<String> = c["participants"]
                                .as_array()
                                .map(|list| list.iter().filter(|p| p["speaking"] == true).map(|p| p["id"].to_string()).collect())
                                .unwrap_or_default();
                            let line = format!(
                                "phase {} muted {} epoch {} video mine {} seat {} speaking [{}] seats {}",
                                c["phase"].as_str().unwrap_or("?"),
                                c["muted"],
                                c["epoch"],
                                c["video_local"],
                                c["participant"],
                                speaking.join(" "),
                                seats.join(" | ")
                            );
                            if line != last_line {
                                println!("{} group_call.state {line} (+{:.2} s)", stamp(), since.elapsed().as_secs_f32());
                                last_line = line;
                            }
                            // Where I sit: through my own node or on the
                            // home itself, and the room moving to another
                            // node when its home died (cascade.md).
                            if let Some(moved) = way.moved(c["node"].as_str().unwrap_or(""), c["home"].as_str().unwrap_or("")) {
                                println!("{} group_call.state {moved} (+{:.2} s)", stamp(), since.elapsed().as_secs_f32());
                            }
                            match c["phase"].as_str() {
                                Some("reconnecting") if lost_at.is_none() => {
                                    lost_at = Some(std::time::Instant::now());
                                    println!("{} group_call.state the way to the room is lost: reconnecting (+{:.2} s)", stamp(), since.elapsed().as_secs_f32());
                                }
                                Some("in_room") => {
                                    if let Some(t) = lost_at.take() {
                                        println!(
                                            "{} group_call.state back in the room after {:.1} s out of it ({}, epoch {}, seat {})",
                                            stamp(),
                                            t.elapsed().as_secs_f32(),
                                            way.here(),
                                            c["epoch"],
                                            c["participant"]
                                        );
                                    }
                                }
                                _ => {}
                            }
                            // The m-lines of the others: whose they are, and
                            // the frames of every video as it appears.
                            if let Some(list) = c["participants"].as_array() {
                                // The videos not watched yet, noted under the
                                // lock; asked for without it.
                                let mut fresh: Vec<(u32, String)> = vec![];
                                {
                                    let mut g = got.lock().unwrap();
                                    for p in list.iter().filter(|p| p["me"] != true) {
                                        let Some(seat) = p["id"].as_u64() else { continue };
                                        for key in ["audio_mid", "video_mid"] {
                                            if let Some(mid) = p[key].as_str() {
                                                g.seats.insert(mid.to_string(), seat as u32);
                                            }
                                        }
                                        if let Some(mid) = p["video_mid"].as_str() {
                                            if !g.video.contains_key(mid) {
                                                let per = Arc::new(std::sync::Mutex::new(VideoGot::default()));
                                                g.video.insert(mid.to_string(), per);
                                                fresh.push((seat as u32, mid.to_string()));
                                            }
                                        }
                                    }
                                }
                                for (seat, mid) in fresh {
                                    if let Some(frames) = rt.group_call_video_frames(&mid).await {
                                        let per = got.lock().unwrap().video.get(&mid).cloned().expect("noted above");
                                        tasks.push(watch_video_of(&format!("seat {seat} mid {mid}"), frames, per));
                                    }
                                }
                            }
                            if c["phase"] == "in_room" && in_room_at.is_none() {
                                in_room_at = Some(std::time::Instant::now());
                                if let Some(secs) = plan.secs {
                                    hang_up.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(secs));
                                }
                            }
                            if c["phase"] == "left" {
                                outcome.get_or_insert(("\"left\"".to_string(), "null".to_string()));
                                break;
                            }
                        }
                        messenger_runtime::UI_EVENT_GROUP_CALL_ENDED => {
                            outcome = Some((e.payload["outcome"].to_string(), e.payload["duration_secs"].to_string()));
                            break;
                        }
                        messenger_runtime::UI_EVENT_GROUP_CALL_STARTED => {
                            let c = &e.payload["call"];
                            println!("{} group_call.started participants {} joined {}", stamp(), c["participants"], c["joined"]);
                        }
                        _ => {}
                    }
                }
                Ok(e) if e.name == "error" && e.payload["scope"] == "group_calls" => {
                    println!("{} error {}", stamp(), e.payload["error"]);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("(lagged {n})"),
                Err(_) => break,
            },
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(line)) => {
                    let words: Vec<&str> = line.split_whitespace().collect();
                    match words.as_slice() {
                        ["leave"] | ["end"] => { let _ = rt.group_call_leave().await.map_err(|e| eprintln!("leave: {e}")); }
                        ["mute"] => { let _ = rt.group_call_set_mute(true).await.map_err(|e| eprintln!("mute: {e}")); }
                        ["unmute"] => { let _ = rt.group_call_set_mute(false).await.map_err(|e| eprintln!("unmute: {e}")); }
                        ["video", "on"] => { let _ = rt.group_call_set_video(VideoInput::Camera { id: None }).await.map_err(|e| eprintln!("video on: {e}")); }
                        ["video", "off"] => { let _ = rt.group_call_set_video(VideoInput::Off).await.map_err(|e| eprintln!("video off: {e}")); }
                        ["layer", seat, rid] => match seat.parse::<u32>() {
                            Ok(seat) => { let _ = rt.group_call_set_layer(seat, rid).await.map_err(|e| eprintln!("layer: {e}")); }
                            Err(_) => eprintln!("layer: a seat is a number"),
                        },
                        ["state"] => println!("{} {}", stamp(), serde_json::to_string(&rt.group_call_state(None).await).unwrap_or_default()),
                        [] => {}
                        _ => eprintln!("(unknown: {line}; leave, mute, unmute, video on, video off, layer <seat> <q|h|f>, state)"),
                    }
                }
                // No stdin (a pipe that closed, /dev/null): the call goes on without it.
                _ => stdin_open = false,
            },
            _ = &mut hang_up => {
                hang_up.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(u64::MAX / 4));
                println!("{} leaving after {} s", stamp(), plan.secs.unwrap_or(0));
                let _ = rt.group_call_leave().await.map_err(|e| eprintln!("leave: {e}"));
            }
            _ = tokio::signal::ctrl_c() => {
                println!("{} Ctrl-C: leaving", stamp());
                let _ = rt.group_call_leave().await.map_err(|e| eprintln!("leave: {e}"));
            }
        }
    }
    if let Some(p) = pump {
        p.abort();
    }
    // A moment for the last frames, then the streams are let go.
    tokio::time::sleep(Duration::from_millis(500)).await;
    for t in tasks {
        t.abort();
    }
    let (outcome, duration) = outcome.unwrap_or_else(|| ("\"?\"".into(), "null".into()));
    println!(
        "{} group call over: outcome {outcome} duration {duration} s{}",
        stamp(),
        in_room_at.map(|a| format!(" (in the room for {:.1} s here)", a.elapsed().as_secs_f32())).unwrap_or_default()
    );
    let got = std::mem::take(&mut *got.lock().unwrap());
    let seat_of = |mid: &str| got.seats.get(mid).map(|s| format!("seat {s}")).unwrap_or_else(|| "seat ?".into());
    let mut written: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for (mid, samples) in &got.audio {
        // One file per seat: `<out>` with the seat before the extension;
        // a seat with a second m-line (the room moved, or I joined it
        // again) gets the m-line after the seat, not the first file over.
        let path = plan.out_path.as_ref().map(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("wav");
            let seat = seat_of(mid).replace(' ', "");
            let first = p.with_file_name(format!("{stem}-{seat}.{ext}"));
            let path = if written.contains(&first) { p.with_file_name(format!("{stem}-{seat}-{mid}.{ext}")) } else { first };
            written.insert(path.clone());
            path
        });
        report_received_of(&format!("{} mid {mid}", seat_of(mid)), samples, path.as_deref());
    }
    if got.audio.is_empty() {
        println!("{} audio received nothing from any seat", stamp());
    }
    for (mid, v) in &got.video {
        let v = v.lock().unwrap();
        println!(
            "{} video received {} mid {mid}: {} frames; sizes (width x height, rotation) {:?}; best second {} fps; the test pattern seen: {}",
            stamp(),
            seat_of(mid),
            v.frames,
            v.sizes,
            v.peak_fps,
            v.pattern
        );
    }
}

/// Where I sit as the state last said: the node I am connected to and
/// the home of the room. A line only when a known way changed: nothing
/// while the call is `starting` and no node is named yet (`gcall` of the
/// creator before the node answered), `node A` when the way is first
/// known, `room moved: …` when the home changed (cascade.md).
#[derive(Default)]
struct Way {
    at: Option<(String, String)>,
}

impl Way {
    /// The line to print for the state that just came, or nothing when
    /// the way is the same, or still unknown (no node named).
    fn moved(&mut self, node: &str, home: &str) -> Option<String> {
        if node.is_empty() {
            // Not connected yet: no way to print, and nothing to forget.
            return None;
        }
        let here = where_of(node, home);
        let line = match &self.at {
            Some((_, old_home)) if !old_home.is_empty() && *old_home != home => Some(format!("room moved: home {old_home} → {here}")),
            // The same line as before (the home named later as my own
            // node): remembered, so that a later move is seen, not repeated.
            Some(_) if self.here() == here => None,
            _ => Some(here),
        };
        self.at = Some((node.to_string(), home.to_string()));
        line
    }

    /// Where I sit now, or empty while unknown.
    fn here(&self) -> String {
        self.at.as_ref().map(|(n, h)| where_of(n, h)).unwrap_or_default()
    }
}

/// Where I sit, on a line: `node A` when the room is on the node I am
/// connected to, `via node B → home A` when I sit in the room of A
/// through my own node B (the cascade); `home` empty before the join
/// was answered, or from a runtime before the cascade.
fn where_of(node: &str, home: &str) -> String {
    if home.is_empty() || home == node {
        format!("node {node}")
    } else {
        format!("via node {node} → home {home}")
    }
}

/// One seat of the room on a line: `#2 npub1abc… ok speaking a:1 v:2 (me)`.
fn seat_line(p: &serde_json::Value) -> String {
    format!(
        "#{} {} {}{}{}{}{}",
        p["id"],
        p["npub"].as_str().map(|n| n[..12].to_string()).unwrap_or_else(|| "?".into()),
        if p["verified"] == true { "ok" } else { "unverified" },
        if p["speaking"] == true { " speaking" } else { "" },
        p["audio_mid"].as_str().map(|m| format!(" a:{m}")).unwrap_or_default(),
        p["video_mid"].as_str().map(|m| format!(" v:{m}")).unwrap_or_default(),
        if p["me"] == true { " (me)" } else { "" },
    )
}

/// The wall clock in milliseconds, for lines compared across processes
/// and machines (the delay of the signalling).
fn stamp() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("[{}.{:03}]", ms / 1000, ms % 1000)
}
