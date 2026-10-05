// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The manager with a real tor, over the real network. Ignored by default:
//! CI has no bundle and need not reach the Tor network. Run with the path
//! of an unpacked Tor Expert Bundle (the directory with `tor/` and `data/`):
//!
//! ```sh
//! VEYDAN_TOR_BUNDLE=/path/to/bundle cargo test -p veydan-tor --locked \
//!     --test real_tor -- --ignored --nocapture --test-threads 1
//! ```

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use veydan_tor::control::Control;
use veydan_tor::{ExitSet, InstanceState, Lease, TorManager, TorSettings};

fn bundle_root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("VEYDAN_TOR_BUNDLE")
            .expect("VEYDAN_TOR_BUNDLE names an unpacked Tor Expert Bundle"),
    )
}

/// A data directory with a space in its path, the bundle linked into it
/// where the app keeps it.
fn data_dir(tmp: &Path) -> PathBuf {
    let data = tmp.join("data dir with space");
    std::fs::create_dir_all(data.join("tor")).unwrap();
    std::os::unix::fs::symlink(bundle_root(), data.join("tor").join("bundle")).unwrap();
    data
}

/// What check.torproject.org says of the way through `lease`.
async fn check(lease: &Lease) -> serde_json::Value {
    let proxy = reqwest::Proxy::all(format!(
        "socks5h://{}:{}@127.0.0.1:{}",
        lease.username, lease.password, lease.socks_port
    ))
    .unwrap();
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let mut last = String::new();
    // A fresh circuit fails at times; tor builds another.
    for _ in 0..3 {
        match client
            .get("https://check.torproject.org/api/ip")
            .send()
            .await
        {
            Ok(response) => {
                let text = response.text().await.unwrap();
                return serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}"));
            }
            Err(e) => last = format!("{e:?}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    panic!("no answer through tor: {last}");
}

/// A second control connection to the instance, for what the test checks.
async fn control(manager: &TorManager, key: &str) -> Control {
    let dir = manager.instance_dir(key);
    Control::connect(&dir.join("control.port"), &dir.join("control.cookie"))
        .await
        .unwrap()
}

async fn pid(manager: &TorManager, key: &str) -> u32 {
    control(manager, key)
        .await
        .get_info("process/pid")
        .await
        .unwrap()
        .parse()
        .unwrap()
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs VEYDAN_TOR_BUNDLE and the network"]
async fn tor_runs_isolates_restarts_and_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let data = data_dir(tmp.path());
    let manager = TorManager::new(data.clone(), TorSettings::defaults(), |_| {});

    // Two consumers at once share one start.
    let started = std::time::Instant::now();
    let any = ExitSet::any();
    let (a, b) = tokio::join!(
        manager.acquire(&any, "test-a"),
        manager.acquire(&any, "test-b")
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    println!("bootstrapped in {:?}, socks port {}", started.elapsed(), a.socks_port);
    assert_eq!(a.socks_port, b.socks_port);
    assert_ne!(a.password, b.password);
    let list = manager.instances();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, InstanceState::Ready);
    assert_eq!(list[0].bootstrap, 100);
    assert_eq!(list[0].consumers, 2);
    assert!(data.join("tor/instances/any/torrc").is_file());
    let torrc = std::fs::read_to_string(data.join("tor/instances/any/torrc")).unwrap();
    assert!(torrc.contains("DataDirectory \""), "{torrc}");
    assert!(torrc.contains("data dir with space"), "{torrc}");

    let answer_a = check(&a).await;
    let answer_b = check(&b).await;
    println!("a: {answer_a}\nb: {answer_b}");
    assert_eq!(answer_a["IsTor"], true);
    assert_eq!(answer_b["IsTor"], true);

    manager.new_identity("any").await.unwrap();
    assert_eq!(check(&a).await["IsTor"], true);

    // The exit in Germany, as tor's own GeoIP database places it.
    let de = manager
        .acquire(&ExitSet::parse("DE").unwrap(), "test-de")
        .await
        .unwrap();
    assert_ne!(de.socks_port, a.socks_port);
    let answer = check(&de).await;
    println!("de: {answer}");
    assert_eq!(answer["IsTor"], true);
    let ip = answer["IP"].as_str().unwrap().to_string();
    let country = control(&manager, "de")
        .await
        .get_info(&format!("ip-to-country/{ip}"))
        .await
        .unwrap();
    println!("the exit {ip} is in {country}");
    assert_eq!(country, "de");

    // tor dies under its consumers: it comes back on the same port.
    let any_pid = pid(&manager, "any").await;
    let de_pid = pid(&manager, "de").await;
    std::process::Command::new("kill")
        .args(["-9", &any_pid.to_string()])
        .status()
        .unwrap();
    let mut seen_restart = false;
    let back = tokio::time::timeout(Duration::from_secs(200), async {
        loop {
            let info = manager
                .instances()
                .into_iter()
                .find(|i| i.key == "any")
                .unwrap();
            seen_restart |= info.state == InstanceState::Restarting;
            if info.state == InstanceState::Ready && seen_restart {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("tor came back");
    assert_eq!(back.socks_port, Some(a.socks_port));
    assert_eq!(check(&b).await["IsTor"], true);
    let new_pid = pid(&manager, "any").await;
    assert_ne!(new_pid, any_pid);
    let log = manager.log("any").unwrap();
    assert!(log.iter().any(|l| l.contains("Bootstrapped 100%")), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("-- tor exited")), "{log:?}");

    // A lease dropped is released.
    drop(b);
    let any = |m: &TorManager| m.instances().into_iter().find(|i| i.key == "any").unwrap();
    assert_eq!(any(&manager).consumers, 1);

    // Everything stops with the app.
    manager.stop_all().await;
    assert!(manager.instances().is_empty());
    assert!(!alive(new_pid), "tor {new_pid} still runs");
    assert!(!alive(de_pid), "tor {de_pid} still runs");
    assert!(manager.acquire(&ExitSet::any(), "late").await.is_err());
    drop((a, de));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs VEYDAN_TOR_BUNDLE"]
async fn an_idle_instance_stops_and_settings_restart_it() {
    let tmp = tempfile::tempdir().unwrap();
    let data = data_dir(tmp.path());
    let mut settings = TorSettings::defaults();
    settings.idle_minutes = 0;
    let manager = TorManager::new(data, settings.clone(), |_| {});

    // Kept by hand, a change of the settings starts it again.
    manager.start(&ExitSet::any()).await.unwrap();
    let ready = |m: TorManager| async move {
        tokio::time::timeout(Duration::from_secs(200), async {
            loop {
                let list = m.instances();
                if list.first().is_some_and(|i| i.state == InstanceState::Ready) {
                    return list[0].clone();
                }
                assert!(
                    !list.first().is_some_and(|i| i.state == InstanceState::Failed),
                    "{list:?}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("ready")
    };
    let first = ready(manager.clone()).await;
    let first_pid = pid(&manager, "any").await;
    settings.extra_torrc = vec!["NumEntryGuards 2".into()];
    manager.apply_settings(settings.clone()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = ready(manager.clone()).await;
    let second_pid = pid(&manager, "any").await;
    assert_ne!(first_pid, second_pid);
    assert!(!alive(first_pid));
    assert_eq!(first.socks_port, second.socks_port);
    assert!(!second.restart_needed);

    // With a consumer, the change waits for the next start.
    let lease = manager.acquire(&ExitSet::any(), "c").await.unwrap();
    settings.extra_torrc.clear();
    manager.apply_settings(settings.clone()).await;
    let info = manager.instances()[0].clone();
    assert!(info.restart_needed, "{info:?}");
    assert_eq!(pid(&manager, "any").await, second_pid);
    assert!(manager.stop("any").unwrap_err().to_string().starts_with("tor_in_use"));

    // Not kept and released, with idle_minutes 0 it goes at once.
    drop(lease);
    tokio::time::timeout(Duration::from_secs(15), async {
        while !manager.instances().is_empty() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the idle instance stopped");
    assert!(!alive(second_pid));
}

/// tor exits by itself when the app is gone: the connection that took
/// ownership closes, or the process named by `__OwningControllerProcess`
/// exits (tor looks every 15 s).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs VEYDAN_TOR_BUNDLE"]
async fn tor_follows_its_owner() {
    use veydan_tor::torrc::{self, TorrcPaths};
    let tmp = tempfile::tempdir().unwrap();
    let bundle = veydan_tor::Bundle::at(&bundle_root());

    let start = |name: &str, owner: u32| {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("data"), std::fs::Permissions::from_mode(0o700)).unwrap();
        let paths = TorrcPaths {
            data_dir: dir.join("data"),
            control_port_file: dir.join("control.port"),
            cookie_file: dir.join("control.cookie"),
            geoip: bundle.geoip.clone(),
            geoip6: bundle.geoip6.clone(),
            pt_path: format!("{}/", bundle.pt_dir.display()),
        };
        let text = torrc::torrc(&TorSettings::defaults(), &ExitSet::any(), None, &paths, owner, None)
            .unwrap()
            + "DisableNetwork 1\n";
        std::fs::write(dir.join("torrc"), text).unwrap();
        std::fs::write(dir.join("defaults"), "").unwrap();
        let child = tokio::process::Command::from(bundle.command())
            .arg("-f")
            .arg(dir.join("torrc"))
            .arg("--defaults-torrc")
            .arg(dir.join("defaults"))
            .stdout(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        (dir, child)
    };
    async fn wait_port(dir: &Path) {
        for _ in 0..100 {
            if dir.join("control.port").is_file() && dir.join("control.cookie").is_file() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no control port");
    }

    // The owner process dies.
    let mut owner = std::process::Command::new("sleep").arg("600").spawn().unwrap();
    let (dir, mut tor) = start("owner", owner.id());
    wait_port(&dir).await;
    owner.kill().unwrap();
    owner.wait().unwrap();
    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(40), tor.wait())
        .await
        .expect("tor exits after its owner")
        .unwrap();
    println!("tor exited {:?} after its owner", started.elapsed());

    // The owning connection closes.
    let (dir, mut tor) = start("connection", std::process::id());
    wait_port(&dir).await;
    let mut control = Control::connect(&dir.join("control.port"), &dir.join("control.cookie"))
        .await
        .unwrap();
    control.take_ownership().await.unwrap();
    drop(control);
    tokio::time::timeout(Duration::from_secs(10), tor.wait())
        .await
        .expect("tor exits when the owning connection closes")
        .unwrap();
}

/// Built-in bridges: the transports start from the bundle, named relative
/// to the working directory of tor. `VEYDAN_TOR_BRIDGE` picks the kind
/// (obfs4 by default).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs VEYDAN_TOR_BUNDLE and the network"]
async fn built_in_bridges_bootstrap() {
    let tmp = tempfile::tempdir().unwrap();
    let data = data_dir(tmp.path());
    let mut settings = TorSettings::defaults();
    settings.bridges.mode = veydan_tor::settings::BridgeMode::Builtin;
    settings.bridges.builtin = std::env::var("VEYDAN_TOR_BRIDGE").unwrap_or_else(|_| "obfs4".into());
    let manager = TorManager::new(data.clone(), settings.validate().unwrap(), |_| {});
    let lease = manager.acquire(&ExitSet::any(), "bridged").await;
    let log = manager.log("any").unwrap_or_default();
    let torrc = std::fs::read_to_string(data.join("tor/instances/any/torrc")).unwrap();
    let lease = lease.unwrap_or_else(|e| panic!("{e}\n{torrc}\n{}", log.join("\n")));
    assert!(torrc.contains("UseBridges 1"));
    assert!(
        torrc.contains("exec bundle/tor/pluggable_transports/lyrebird"),
        "{torrc}"
    );
    assert_eq!(check(&lease).await["IsTor"], true);
    println!(
        "{}",
        log.iter()
            .filter(|l| l.contains("bridge") || l.contains("Bridge") || l.contains("proxy"))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    manager.stop_all().await;
}
