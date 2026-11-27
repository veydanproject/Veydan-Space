// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The extension's session on a computer, against a data folder (a
//! simulator's group container): the chats, then a text sent into one and
//! what the wait sees at every step.
//!   cargo run -p veydan-share-ext --example probe -- <data dir> [chat id] [text] [file…]

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("data dir");
    let rt = veydan_share_ext::runtime().unwrap();
    rt.block_on(async {
        let (_lock, m) = match veydan_share_ext::open(Path::new(dir), None).await {
            Ok(s) => s,
            Err((code, why)) => {
                eprintln!("open: {code} {why}");
                return;
            }
        };
        let chats = m.chats(false).await.unwrap();
        for c in veydan_share_ext::targets(chats) {
            println!("chat {} {} {:?}", c.id, c.title, c.last_message_at);
        }
        let Some(chat) = args.get(2) else { return };
        let recent = m.messages(chat, None, 100).await.unwrap();
        println!("messages of {chat}: {}", recent.len());
        for x in recent.iter().take(3) {
            println!("  {} {} {} {}", x.id, x.status, x.content_type, x.created_at);
        }
        let text = args.get(3).map(String::as_str);
        let files: Vec<std::path::PathBuf> = args.iter().skip(4).map(std::path::PathBuf::from).collect();
        if text.is_none() && files.is_empty() {
            return;
        }
        let started = std::time::Instant::now();
        let mut report = |stage: i32, done: u64, total: u64| println!("[{:>5.1}s] stage {stage} {done}/{total}", started.elapsed().as_secs_f32());
        match veydan_share_ext::send(&m, chat, text, &files, &mut report).await {
            Ok(()) => println!("sent"),
            Err(e) => println!("failed: {e}"),
        }
        m.shutdown().await;
    });
}
