//! What both renderers share: the rows, the meter of section 9, and the picture diff.
pub mod meter;
pub mod picture;

pub const ROWS: usize = 100_000;
pub const ROW_H: f64 = 52.0;
pub const HEADER_H: f64 = 56.0;
/// One viewport a frame, so every frame draws rows it has never drawn.
pub const SCROLL_STEP: f64 = 700.0;
pub const INTER: &[u8] = include_bytes!("../../fonts/Inter-VariableFont.ttf");
/// design.json's sans: the design page's one face. A window loads one font (build_single_font_ctx), so mono and hand fall back to it.
pub const DESIGN_FONT: &[u8] = include_bytes!("../../fonts/InstrumentSans-latin.woff2");
/// The window sizes of section 9's picture diff.
pub const SIZES: [(u32, u32); 3] = [(400, 700), (800, 600), (1200, 800)];
/// Scroll offset of the mid-list picture: row 50,000 at the top, so a picture proves the list is virtual.
pub const MID_SCROLL: f64 = 50_000.0 * ROW_H;

pub struct Row {
    pub title: String,
    pub subtitle: String,
    pub initials: &'static str,
    pub badge: &'static str,
    pub tone: Tone,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Outline,
    Destructive,
}

const OWNERS: [(&str, &str); 4] = [
    ("NH", "Nicolas"),
    ("CL", "Claude"),
    ("OP", "Opus"),
    ("PM", "Reviewer"),
];
const BADGES: [(&str, Tone); 4] = [
    ("open", Tone::Primary),
    ("waiting", Tone::Secondary),
    ("review", Tone::Outline),
    ("failed", Tone::Destructive),
];

/// Computed from the index, never stored: the numbers measure the renderer, not a copy of the data.
pub fn row(i: usize) -> Row {
    let (initials, owner) = OWNERS[i % OWNERS.len()];
    let (badge, tone) = BADGES[(i / 3) % BADGES.len()];
    Row {
        title: format!("Row {i}: board item"),
        subtitle: format!("{owner}, {} min ago", i % 60),
        initials,
        badge,
        tone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_stable_and_cover_every_tone() {
        assert_eq!(row(50_000).title, "Row 50000: board item");
        for tone in [
            Tone::Primary,
            Tone::Secondary,
            Tone::Outline,
            Tone::Destructive,
        ] {
            assert!((0..12).any(|i| row(i).tone == tone));
        }
    }
}
