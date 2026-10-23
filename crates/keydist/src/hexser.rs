// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Bytes as lowercase hex in serde: the epoch event and the saved states
//! stay readable JSON. `serialize`/`deserialize` take any byte container
//! of a fixed or free length; `opt` and `list` wrap them.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &T, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(v.as_ref()))
}

pub fn deserialize<'de, D: Deserializer<'de>, T: TryFrom<Vec<u8>>>(d: D) -> Result<T, D::Error> {
    let s = String::deserialize(d)?;
    let v = hex::decode(&s).map_err(D::Error::custom)?;
    T::try_from(v).map_err(|_| D::Error::custom("bytes of the wrong length"))
}

/// An optional byte string.
pub mod opt {
    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_some(&hex::encode(v.as_ref())),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: TryFrom<Vec<u8>>>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(None),
            Some(s) => {
                let v = hex::decode(&s).map_err(D::Error::custom)?;
                T::try_from(v)
                    .map(Some)
                    .map_err(|_| D::Error::custom("bytes of the wrong length"))
            }
        }
    }
}

/// A list of byte strings.
pub mod list {
    use super::*;

    pub fn serialize<S: Serializer, T: AsRef<[u8]>>(v: &[T], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(|x| hex::encode(x.as_ref())))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: TryFrom<Vec<u8>>>(d: D) -> Result<Vec<T>, D::Error> {
        Vec::<String>::deserialize(d)?
            .into_iter()
            .map(|s| {
                let v = hex::decode(&s).map_err(D::Error::custom)?;
                T::try_from(v).map_err(|_| D::Error::custom("bytes of the wrong length"))
            })
            .collect()
    }
}
