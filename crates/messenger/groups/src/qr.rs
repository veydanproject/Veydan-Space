// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A group link as a QR code. The picture is drawn here so that every
//! host shows the same one; reading a code is the host's business.

use messenger_core::{MessengerError, Result};
use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};

/// Links are a few hundred characters; more than this is not a link.
pub const MAX_QR_CHARS: usize = 1_200;

/// SVG of the code: dark modules on a light ground, with the quiet zone,
/// whatever the theme of the host (a code must stay readable).
pub fn link_svg(link: &str) -> Result<String> {
    if link.is_empty() || link.len() > MAX_QR_CHARS {
        return Err(MessengerError::Invalid("group_link_too_long".into()));
    }
    let code = QrCode::with_error_correction_level(link.as_bytes(), EcLevel::M)
        .map_err(|e| MessengerError::Invalid(format!("qr: {e}")))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_becomes_a_picture() {
        let link = format!("veydan://group/{}?t=public&r=wss%3A%2F%2Frelay.example&o={}&n=Square&s={}&e=0", "ab".repeat(32), "cd".repeat(32), "A".repeat(43));
        let svg = link_svg(&link).unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("#000000") && svg.contains("#ffffff"));
        assert!(link_svg("").is_err());
        assert!(link_svg(&"x".repeat(MAX_QR_CHARS + 1)).is_err());
    }
}
