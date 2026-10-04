// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the sync code of 4.0.7 wrote of the entities of notes, kept in
//! `crates/sync-host/tests/golden/` (spec 9.3).

use std::path::Path;
use veydan_sync::{Hlc, Op};

/// Every op 4.0.7 wrote of `entity`, in the order a pull hands them over.
pub(crate) fn ops(entity: &str) -> Vec<Op> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../sync-host/tests/golden/{entity}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let values: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    values
        .iter()
        .map(|value| Op {
            entity_type: value["entity_type"].as_str().unwrap().into(),
            entity_id: value["entity_id"].as_str().unwrap().into(),
            hlc: Hlc::decode(value["hlc"].as_str().unwrap()).expect("hlc"),
            deleted: value["deleted"].as_bool().unwrap(),
            payload: value["payload"].clone(),
        })
        .collect()
}
