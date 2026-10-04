// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/// What a link leads to. A well-formed link of a type this version does
/// not know is still a link: a newer client wrote it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LinkType {
    Group,
    Contact,
    /// A VLink bridge to the project's servers, written `vlink`. The word
    /// `bridge` is kept for bridges to other networks: this version does
    /// not know `veydan://bridge/…`.
    Bridge,
    Unknown(String),
}

impl LinkType {
    /// `word` is already known to be small latin letters.
    pub(crate) fn of(word: &str) -> Self {
        match word {
            "group" => Self::Group,
            "contact" => Self::Contact,
            "vlink" => Self::Bridge,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Group => "group",
            Self::Contact => "contact",
            Self::Bridge => "vlink",
            Self::Unknown(w) => w,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vlink_bridge_is_written_vlink() {
        assert_eq!(LinkType::of("vlink"), LinkType::Bridge);
        assert_eq!(LinkType::Bridge.as_str(), "vlink");
        // `bridge` is kept for bridges to other networks; nothing was
        // released with it, so it is no alias.
        assert_eq!(LinkType::of("bridge"), LinkType::Unknown("bridge".into()));
    }
}
