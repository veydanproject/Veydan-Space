// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the sync code of 4.0.7 wrote, kept in `crates/sync-host/tests/golden/`
//! (spec 9.3): the tests of each type's owner read the ops of their types
//! here; `compat_tests` describes the files.

use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use veydan_sync::{Hlc, Op};

pub(crate) fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/sync-host/tests/golden")
}

pub(crate) fn read_json(name: &str) -> Value {
    let path = dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap()
}

fn op_of(value: &Value) -> Op {
    Op {
        entity_type: value["entity_type"].as_str().unwrap().into(),
        entity_id: value["entity_id"].as_str().unwrap().into(),
        hlc: Hlc::decode(value["hlc"].as_str().unwrap()).expect("hlc"),
        deleted: value["deleted"].as_bool().unwrap(),
        payload: value["payload"].clone(),
    }
}

/// Every op 4.0.7 wrote of `entity`, in the order a pull hands them over.
pub(crate) fn ops(entity: &str) -> Vec<Op> {
    read_json(&format!("{entity}.json"))
        .as_array()
        .unwrap()
        .iter()
        .map(op_of)
        .collect()
}

/// The entity types of the files, one file each.
pub(crate) fn types() -> BTreeSet<String> {
    std::fs::read_dir(dir())
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let entity = name.strip_suffix(".json")?;
            (!matches!(entity, "apply" | "collect" | "vault")).then(|| entity.to_string())
        })
        .collect()
}

/// The entity types 5.x added. 4.0.7 skips them and none of its files holds
/// them; they keep no blob of format v1 (spec 9.5).
pub(crate) const NEW_IN_5: [&str; 1] = [veydan_sync_host::LABEL_ENTITY];
