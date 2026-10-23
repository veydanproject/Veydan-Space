// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `communities-bench`: the keeper and a member at N readers over the
//! fakes of the tests and a real SQLite file (`communities_core::bench`).
//!
//! ```text
//! communities-bench --n 1_000_000 --dir data/bench-1m [--no-from-join] [--catch-up 288]
//! ```

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match rt.block_on(communities_core::bench::run(&args)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("communities-bench: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
