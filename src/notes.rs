//! Decorative music notes around the edge of the slides and the video frames.
//!
//! The notes are vector outlines (no font needed), placed in the side margins that the text never uses, so the same
//! arrangement appears on every slide of a deck and every screen of a video. Positions are on the 1280x720 layout
//! canvas; the deck and the video scale them to their own size.

use crate::theme::{hex_to_rgb, Theme};

/// A piece of an outline, in canvas pixels (y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
}

/// A closed part of a note (head, stem, flag, beam). Parts may overlap, so each is filled on its own.
pub type Part = Vec<Seg>;

#[derive(Clone, Copy)]
enum Glyph {
    Quarter,
    Eighth,
    Beamed,
}

/// (glyph, center x, center y, height, rotation in degrees). The side bands (x < 62 and x > 1218) stay clear of the
/// verse text (70 px margins) and, above y 130, of the title strip (40 px margins), where the notes are smaller.
const PLACEMENTS: [(Glyph, f64, f64, f64, f64); 10] = [
    (Glyph::Eighth, 21.0, 62.0, 30.0, -10.0),
    (Glyph::Eighth, 32.0, 210.0, 52.0, -12.0),
    (Glyph::Beamed, 33.0, 365.0, 44.0, 8.0),
    (Glyph::Quarter, 30.0, 515.0, 50.0, -6.0),
    (Glyph::Eighth, 32.0, 655.0, 42.0, 14.0),
    (Glyph::Beamed, 1259.0, 68.0, 28.0, 10.0),
    (Glyph::Beamed, 1247.0, 180.0, 44.0, 12.0),
    (Glyph::Eighth, 1248.0, 330.0, 52.0, -10.0),
    (Glyph::Quarter, 1250.0, 480.0, 42.0, 8.0),
    (Glyph::Beamed, 1246.0, 630.0, 46.0, -12.0),
];

/// How strongly the note color (the title color) shows over the background
const OPACITY: f64 = 0.45;

/// Cubic Bezier constant for a quarter ellipse
const KAPPA: f64 = 0.552_284_75;

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64, angle_deg: f64) -> Part {
    let (s, c) = angle_deg.to_radians().sin_cos();
    let p = |x: f64, y: f64| (cx + x * c - y * s, cy + x * s + y * c);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let pts = [
        [p(rx, ky), p(kx, ry), p(0.0, ry)],
        [p(-kx, ry), p(-rx, ky), p(-rx, 0.0)],
        [p(-rx, -ky), p(-kx, -ry), p(0.0, -ry)],
        [p(kx, -ry), p(rx, -ky), p(rx, 0.0)],
    ];
    let start = p(rx, 0.0);
    let mut part = vec![Seg::Move(start.0, start.1)];
    for [a, b, e] in pts {
        part.push(Seg::Cubic(a.0, a.1, b.0, b.1, e.0, e.1));
    }
    part
}

fn polygon(points: &[(f64, f64)]) -> Part {
    let mut part = vec![Seg::Move(points[0].0, points[0].1)];
    part.extend(points[1..].iter().map(|&(x, y)| Seg::Line(x, y)));
    part
}

/// A glyph's parts in a unit box (x right, y down, both 0..1; the box is centered on the note).
fn glyph_parts(glyph: Glyph) -> Vec<Part> {
    match glyph {
        Glyph::Quarter => vec![
            ellipse(0.40, 0.84, 0.19, 0.13, -20.0),
            polygon(&[(0.515, 0.06), (0.585, 0.06), (0.585, 0.82), (0.515, 0.84)]),
        ],
        Glyph::Eighth => vec![
            ellipse(0.30, 0.84, 0.19, 0.13, -20.0),
            polygon(&[(0.415, 0.04), (0.485, 0.04), (0.485, 0.82), (0.415, 0.84)]),
            // The flag: out from the top of the stem, curling down
            vec![
                Seg::Move(0.415, 0.04),
                Seg::Line(0.485, 0.04),
                Seg::Cubic(0.50, 0.20, 0.82, 0.26, 0.74, 0.62),
                Seg::Cubic(0.72, 0.42, 0.60, 0.33, 0.415, 0.30),
            ],
        ],
        Glyph::Beamed => vec![
            ellipse(0.19, 0.86, 0.17, 0.115, -20.0),
            ellipse(0.73, 0.78, 0.17, 0.115, -20.0),
            polygon(&[(0.29, 0.14), (0.35, 0.14), (0.35, 0.84), (0.29, 0.86)]),
            polygon(&[(0.83, 0.06), (0.89, 0.06), (0.89, 0.76), (0.83, 0.78)]),
            // The beam joining the stem tops
            polygon(&[(0.29, 0.10), (0.89, 0.0), (0.89, 0.14), (0.29, 0.24)]),
        ],
    }
}

/// Every note's parts, placed on the 1280x720 canvas.
pub fn frame_notes() -> Vec<Vec<Part>> {
    PLACEMENTS
        .iter()
        .map(|&(glyph, cx, cy, size, angle)| {
            let (s, c) = angle.to_radians().sin_cos();
            let place = |x: f64, y: f64| {
                let (ux, uy) = ((x - 0.5) * size, (y - 0.5) * size);
                (cx + ux * c - uy * s, cy + ux * s + uy * c)
            };
            glyph_parts(glyph)
                .into_iter()
                .map(|part| {
                    part.into_iter()
                        .map(|seg| match seg {
                            Seg::Move(x, y) => {
                                let p = place(x, y);
                                Seg::Move(p.0, p.1)
                            }
                            Seg::Line(x, y) => {
                                let p = place(x, y);
                                Seg::Line(p.0, p.1)
                            }
                            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                                let (a, b, e) = (place(x1, y1), place(x2, y2), place(x, y));
                                Seg::Cubic(a.0, a.1, b.0, b.1, e.0, e.1)
                            }
                        })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// Bounding box (left, top, right, bottom) of a note's points, control points included.
pub fn bounds(parts: &[Part]) -> (f64, f64, f64, f64) {
    let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut add = |x: f64, y: f64| b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
    for seg in parts.iter().flatten() {
        match *seg {
            Seg::Move(x, y) | Seg::Line(x, y) => add(x, y),
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                add(x1, y1);
                add(x2, y2);
                add(x, y);
            }
        }
    }
    b
}

/// The notes' color: the title color softened toward the background, as an opaque #RRGGBB (so overlapping parts of
/// a note don't show darker, in the deck as in the video).
pub fn note_color(theme: &Theme) -> String {
    let (t, b) = (hex_to_rgb(&theme.title_color), hex_to_rgb(&theme.bg_color));
    let mix = |t: u8, b: u8| (b as f64 + (t as f64 - b as f64) * OPACITY).round() as u8;
    format!("#{:02X}{:02X}{:02X}", mix(t.0, b.0), mix(t.1, b.1), mix(t.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_stay_in_the_side_bands() {
        for note in frame_notes() {
            let (l, t, r, b) = bounds(&note);
            assert!(t >= 0.0 && b <= 720.0, "{:?}", (l, t, r, b));
            let band = if t < 130.0 { 40.0 } else { 70.0 };
            assert!(r < band || l > 1280.0 - band, "{:?}", (l, t, r, b));
        }
    }

    #[test]
    fn color_mix() {
        let theme = Theme { bg_color: "#000000".into(), text_color: "#FFFFFF".into(), title_color: "#FFFFFF".into() };
        assert_eq!(note_color(&theme), "#737373");
    }
}
