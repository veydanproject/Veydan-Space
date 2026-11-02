// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A link as a QR code, the one picture of every code the app shows
//! (ROO-462): rounded modules that run together, rounded finder "eyes",
//! a white rounded card with the quiet zone, and a dark disc in the
//! middle where the host puts the logo of its product. The disc takes the
//! modules under it away, so the code is made at level H, which reads
//! with up to 30 % of it lost; the disc is a little over 6 %.
//!
//! The disc is `<circle class="logo" cx cy r>` in the units of the
//! `viewBox`: the host finds it there (ui/src/lib/core/ui/qrPicture.ts).

use qrcode::{Color, EcLevel, QrCode};
use std::fmt::Write as _;

/// The color of the modules; the card is white, whatever the theme of the host.
pub const DARK: &str = "#1f2328";
/// The quiet zone around the code, in modules: the standard's 4 at least,
/// a tenth of the code on a big one, so that the card frames it.
fn quiet(n: usize) -> usize {
    4.max(n.div_ceil(10))
}
/// The radius of a free corner of a module.
const CORNER: f64 = 0.4;
/// The radii of an eye's corners: its ring, the hole in it, the pupil.
const EYE: [f64; 3] = [1.75, 1.1, 0.65];
/// The radius of the free middle, as a share of the side of the code.
const HOLE: f64 = 0.14;
/// Pixels per module of the picture's own size; the hosts scale it.
const PX: usize = 8;

/// What a picture is drawn of, in modules; the card's corner is at 0, 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A rectangle, its corners rounded by `r`: top left, top right, bottom right, bottom left.
    Rect { x: f64, y: f64, w: f64, h: f64, r: [f64; 4], dark: bool },
    /// The disc of the logo.
    Disc { cx: f64, cy: f64, r: f64 },
}

/// The code of `text` at level H; `None` when it does not fit in one.
fn code(text: &str) -> Option<QrCode> {
    QrCode::with_error_correction_level(text.as_bytes(), EcLevel::H).ok()
}

/// The shapes of the code of `text`, painted in order, and the side of the card.
pub fn shapes(text: &str) -> Option<(Vec<Shape>, f64)> {
    let code = code(text)?;
    let n = code.width();
    let colors = code.to_colors();
    let q = quiet(n);
    let side = (n + 2 * q) as f64;
    let mid = n as f64 / 2.0;
    let hole = n as f64 * HOLE;
    let eye = |x: usize, y: usize| (x < 7 || x >= n - 7) && y < 7 || x < 7 && y >= n - 7;
    let free = |x: usize, y: usize| {
        let (dx, dy) = (x as f64 + 0.5 - mid, y as f64 + 0.5 - mid);
        dx * dx + dy * dy < hole * hole
    };
    // A module drawn as such: dark, not of an eye, not under the disc.
    let on = |x: isize, y: isize| {
        if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
            return false;
        }
        let (x, y) = (x as usize, y as usize);
        colors[y * n + x] == Color::Dark && !eye(x, y) && !free(x, y)
    };
    let q = q as f64;
    // The corner of the card keeps off the eyes: its arc comes in 0.41 r along the diagonal.
    let mut out = vec![Shape::Rect { x: 0.0, y: 0.0, w: side, h: side, r: [q * 0.9; 4], dark: false }];
    // A row's run of modules is one rectangle: a corner of an end is round
    // where neither the module beside it nor the one above (below) is drawn.
    for y in 0..n as isize {
        let mut x = 0isize;
        while x < n as isize {
            if !on(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while on(x + 1, y) {
                x += 1;
            }
            let r = [
                if !on(start - 1, y) && !on(start, y - 1) { CORNER } else { 0.0 },
                if !on(x + 1, y) && !on(x, y - 1) { CORNER } else { 0.0 },
                if !on(x + 1, y) && !on(x, y + 1) { CORNER } else { 0.0 },
                if !on(start - 1, y) && !on(start, y + 1) { CORNER } else { 0.0 },
            ];
            out.push(Shape::Rect { x: q + start as f64, y: q + y as f64, w: (x - start + 1) as f64, h: 1.0, r, dark: true });
            x += 1;
        }
    }
    for (ex, ey) in [(0, 0), (n - 7, 0), (0, n - 7)] {
        let (ex, ey) = (q + ex as f64, q + ey as f64);
        out.push(Shape::Rect { x: ex, y: ey, w: 7.0, h: 7.0, r: [EYE[0]; 4], dark: true });
        out.push(Shape::Rect { x: ex + 1.0, y: ey + 1.0, w: 5.0, h: 5.0, r: [EYE[1]; 4], dark: false });
        out.push(Shape::Rect { x: ex + 2.0, y: ey + 2.0, w: 3.0, h: 3.0, r: [EYE[2]; 4], dark: true });
    }
    out.push(Shape::Disc { cx: q + mid, cy: q + mid, r: hole - 0.75 });
    Some((out, side))
}

/// A number of the picture: at most three decimals, no trailing zeros, no leading zero.
fn num(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "" | "-0" => "0".into(),
        _ if s.starts_with("0.") => s[1..].into(),
        _ if s.starts_with("-0.") => format!("-{}", &s[2..]),
        _ => s.into(),
    }
}

/// The path of a rectangle with rounded corners, clockwise from the top left.
fn rect_path(d: &mut String, x: f64, y: f64, w: f64, h: f64, [tl, tr, br, bl]: [f64; 4]) {
    let arc = |d: &mut String, r: f64, dx: f64, dy: f64| {
        if r > 0.0 {
            let _ = write!(d, "a{} {} 0 0 1 {} {}", num(r), num(r), num(dx * r), num(dy * r));
        }
    };
    let _ = write!(d, "M{} {}h{}", num(x + tl), num(y), num(w - tl - tr));
    arc(d, tr, 1.0, 1.0);
    let _ = write!(d, "v{}", num(h - tr - br));
    arc(d, br, -1.0, 1.0);
    let _ = write!(d, "h{}", num(-(w - br - bl)));
    arc(d, bl, -1.0, -1.0);
    let _ = write!(d, "v{}", num(-(h - bl - tl)));
    arc(d, tl, 1.0, -1.0);
    d.push('z');
}

/// The SVG of the code of `text`; `None` when it does not fit in one.
pub fn svg(text: &str) -> Option<String> {
    let (shapes, side) = shapes(text)?;
    let px = side as usize * PX;
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {0} {0}" width="{1}" height="{1}">"#,
        num(side),
        px
    );
    // The modules, one path: rectangles side by side show no seam.
    let mut modules = String::new();
    for shape in &shapes {
        match *shape {
            Shape::Rect { x, y, w, h, r, dark } if dark && h == 1.0 => rect_path(&mut modules, x, y, w, h, r),
            Shape::Rect { x, y, w, h, r, dark } => {
                if !modules.is_empty() {
                    let _ = write!(s, r#"<path fill="{DARK}" d="{modules}"/>"#);
                    modules.clear();
                }
                let fill = if dark { DARK } else { "#ffffff" };
                let _ = write!(s, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{fill}"/>"#, num(x), num(y), num(w), num(h), num(r[0]));
            }
            Shape::Disc { cx, cy, r } => {
                if !modules.is_empty() {
                    let _ = write!(s, r#"<path fill="{DARK}" d="{modules}"/>"#);
                    modules.clear();
                }
                let _ = write!(s, r##"<circle class="logo" cx="{}" cy="{}" r="{}" fill="{DARK}"/>"##, num(cx), num(cy), num(r));
            }
        }
    }
    s.push_str("</svg>");
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture as a reader sees it: `k` pixels per module, what is off
    /// the card black (a PNG's clear corners read as black), the disc
    /// covered by `logo` (true: dark). The eyes are square: rqrr, as quirc,
    /// fits the grid to their outer corners, where the readers of the app
    /// (jsQR of «from a picture», ML Kit of the camera) and of phones take
    /// their centres; the rounded eyes are read by jsQR in the test of
    /// ui/src/lib/core/ui/qrPicture.test.ts, on the picture of `FIXTURE`.
    fn raster(text: &str, k: usize, logo: impl Fn(usize, usize) -> bool) -> (Vec<u8>, usize) {
        let (shapes, side) = shapes(text).unwrap();
        let w = side as usize * k;
        let mut px = vec![0u8; w * w];
        // Each shape over the pixels of its box, in order.
        let mut paint = |x: f64, y: f64, sw: f64, sh: f64, v: &dyn Fn(usize, usize, f64, f64) -> Option<u8>| {
            let (x0, y0) = ((x * k as f64) as usize, (y * k as f64) as usize);
            let (x1, y1) = ((((x + sw) * k as f64).ceil() as usize).min(w), (((y + sh) * k as f64).ceil() as usize).min(w));
            for py in y0..y1 {
                for pxx in x0..x1 {
                    if let Some(v) = v(pxx, py, (pxx as f64 + 0.5) / k as f64, (py as f64 + 0.5) / k as f64) {
                        px[py * w + pxx] = v;
                    }
                }
            }
        };
        for shape in &shapes {
            match *shape {
                Shape::Rect { x, y, w, h, r, dark } => {
                    let r = if w == h && (3.0..=7.0).contains(&w) { [0.0; 4] } else { r };
                    paint(x, y, w, h, &|_, _, fx, fy| inside(fx, fy, x, y, w, h, r).then_some(if dark { 0 } else { 255 }));
                }
                Shape::Disc { cx, cy, r } => {
                    let covered = |fx: f64, fy: f64| (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r;
                    paint(cx - r, cy - r, 2.0 * r, 2.0 * r, &|pxx, py, fx, fy| covered(fx, fy).then(|| if logo(pxx, py) { 0 } else { 255 }));
                }
            }
        }
        (px, w)
    }

    fn inside(fx: f64, fy: f64, x: f64, y: f64, w: f64, h: f64, [tl, tr, br, bl]: [f64; 4]) -> bool {
        if fx < x || fy < y || fx > x + w || fy > y + h {
            return false;
        }
        let corner = |cx: f64, cy: f64, r: f64| r == 0.0 || (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r;
        if fx < x + tl && fy < y + tl {
            return corner(x + tl, y + tl, tl);
        }
        if fx > x + w - tr && fy < y + tr {
            return corner(x + w - tr, y + tr, tr);
        }
        if fx > x + w - br && fy > y + h - br {
            return corner(x + w - br, y + h - br, br);
        }
        if fx < x + bl && fy > y + h - bl {
            return corner(x + bl, y + h - bl, bl);
        }
        true
    }

    fn read(px: Vec<u8>, w: usize) -> String {
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, w, |x, y| px[y * w + x]);
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "one code in the picture");
        grids[0].decode().unwrap().1
    }

    fn links() -> Vec<String> {
        vec![
            "veydan://link/ab/abandon".into(),
            format!("veydan://group/{}?t=public&r=wss%3A%2F%2Frelay.example&o={}&n=Square&s={}&e=0", "ab".repeat(32), "cd".repeat(32), "A".repeat(43)),
            format!("veydan://channel/{}?r=wss%3A%2F%2Frelay.example&o={}&s={}&e=0", "ab".repeat(32), "cd".repeat(32), "A".repeat(43)),
            format!("veydan://contact/{}?i={}&srv={}", "ef".repeat(32), "B".repeat(43), "C".repeat(300)),
        ]
    }

    #[test]
    fn a_code_with_the_logo_reads_back_as_its_link() {
        for link in links() {
            // A logo of stripes: dark and light under the disc.
            let (px, w) = raster(&link, 6, |x, y| (x / 3 + y / 5) % 2 == 0);
            assert_eq!(read(px, w), link);
            // The worst a logo can do: the whole disc dark.
            let (px, w) = raster(&link, 6, |_, _| true);
            assert_eq!(read(px, w), link);
        }
    }

    #[test]
    fn the_picture_has_its_card_eyes_and_disc() {
        let link = &links()[1];
        let svg = svg(link).unwrap();
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert_eq!(svg.matches(r#"<circle class="logo""#).count(), 1);
        // The card and, per eye, its ring, hole and pupil.
        assert_eq!(svg.matches("<rect").count(), 1 + 3 * 3);
        assert!(svg.contains(DARK) && svg.contains("#ffffff"));
        // Rounded runs, not squares of pixels.
        assert!(svg.contains("a.4 .4 0 0 1 .4 .4"));
        assert!(svg.len() < 128 * 1024, "{} bytes", svg.len());
    }

    /// A link and its picture, the first and the second line: what the
    /// test of the UI reads with jsQR, as the app reads a saved code.
    const FIXTURE: &str = include_str!("../testdata/qr.txt");

    #[test]
    fn the_picture_of_the_ui_test_is_the_picture_drawn() {
        let link = &links()[1];
        let now = format!("{link}\n{}\n", svg(link).unwrap());
        if FIXTURE != now && std::env::var_os("QR_FIXTURE_WRITE").is_some() {
            std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/qr.txt"), &now).unwrap();
            return;
        }
        assert!(FIXTURE == now, "testdata/qr.txt is of another drawing: run this test again with QR_FIXTURE_WRITE=1");
    }

    #[test]
    fn the_disc_stays_in_the_middle_off_the_eyes() {
        for link in links() {
            let (shapes, side) = shapes(&link).unwrap();
            let Some(Shape::Disc { cx, cy, r }) = shapes.last().copied() else { panic!("no disc") };
            assert_eq!((cx, cy), (side / 2.0, side / 2.0));
            // Clear of the eyes, which end 7 modules into the code.
            assert!(cx - r > quiet(code(&link).unwrap().width()) as f64 + 7.0, "{link}");
            // No module under it.
            for s in &shapes {
                if let Shape::Rect { x, y, w, h, dark: true, .. } = *s {
                    let nx = cx.clamp(x, x + w);
                    let ny = cy.clamp(y, y + h);
                    assert!((nx - cx).powi(2) + (ny - cy).powi(2) >= r * r, "{link}");
                }
            }
        }
    }

    #[test]
    fn what_does_not_fit_has_no_code() {
        assert!(svg(&"x".repeat(4000)).is_none());
    }
}
