// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Smoke test: spawn a ksni StatusNotifierItem and keep it alive briefly so an
//! external check (busctl / the SNI watcher) can confirm it registered.
//! Verifies ksni + zbus work under our tokio setup on this session bus.

#[cfg(target_os = "linux")]
mod smoke {
    use ksni::menu::{MenuItem, StandardItem};
    use ksni::{Category, Status, Tray};

    pub struct Smoke;

    impl Tray for Smoke {
        fn id(&self) -> String {
            "veydan-smoke-tray".into()
        }
        fn title(&self) -> String {
            "Veydan Smoke".into()
        }
        fn category(&self) -> Category {
            Category::ApplicationStatus
        }
        fn status(&self) -> Status {
            Status::Active
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![StandardItem {
                label: "Hello".into(),
                activate: Box::new(|_: &mut Self| println!("activated")),
                ..Default::default()
            }
            .into()]
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            println!("LEFT-CLICK activate() called");
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    use ksni::TrayMethods;
    let handle = smoke::Smoke.spawn().await.expect("ksni spawn failed");
    println!("SPAWNED_OK");
    // Keep the service alive so the external busctl check can see it.
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    handle.shutdown().await;
    println!("DONE");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("linux-only smoke");
}
