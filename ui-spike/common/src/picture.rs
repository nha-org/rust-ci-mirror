//! The picture diff of section 9, replicator's way: one shared reference per screen, a score per zone, a failure past
//! a threshold. The references in rust/ui-spike/refs are the design system: a restyle is a new reference, reviewed as
//! pictures, written by UPDATE_UI_REFS=1 cargo test -p blitz-list (Blitz owns them: real CSS, the web surface's look).
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits, opaque.
    pub rgba: Vec<u8>,
}

pub struct Zone {
    pub name: &'static str,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The zones of the list screen: chrome first, then the row's three columns.
pub fn zones(width: u32, height: u32) -> Vec<Zone> {
    let top = crate::HEADER_H as u32;
    let body = height - top;
    let badge_w = 100.min(width / 3);
    vec![
        Zone {
            name: "header",
            x: 0,
            y: 0,
            w: width,
            h: top,
        },
        Zone {
            name: "avatars",
            x: 0,
            y: top,
            w: 56,
            h: body,
        },
        Zone {
            name: "text",
            x: 56,
            y: top,
            w: width - 56 - badge_w,
            h: body,
        },
        Zone {
            name: "badges",
            x: width - badge_w,
            y: top,
            w: badge_w,
            h: body,
        },
    ]
}

const BLOCK: u32 = 8;
/// Largest difference of a block's mean colour, per channel of 255, that still counts as the same.
const BLOCK_TOLERANCE: f64 = 10.0;
/// A zone under this fails. Measured 2026-11-05 at the tolerance above, content blocks only: Slint's true render scores
/// 0.983 at worst against Blitz's references. Its mutants, best zone: a title 4 px right 0.923, a 28 px avatar 0.909, a
/// wrong badge 0.864, the list off by 12 rows 0.945; one row's title missing fails at 0.973 where that row shows.
pub const THRESHOLD: f64 = 0.98;

/// Share of the zone's content blocks (8x8, past the reference's background in either picture) whose mean colour matches.
/// Antialiasing differs between rasterizers and moves single pixels; a misplaced row, a wrong colour or a missing badge
/// moves whole blocks. Empty background stays out of the count: it hid a whole missing row at 1200x800.
pub fn score(a: &Picture, b: &Picture, z: &Zone) -> f64 {
    let background = [b.rgba[0], b.rgba[1], b.rgba[2]];
    let (mut same, mut total) = (0u32, 0u32);
    let mut y = z.y;
    while y < z.y + z.h {
        let mut x = z.x;
        while x < z.x + z.w {
            if has_content(a, background, x, y, z) || has_content(b, background, x, y, z) {
                let (ma, mb) = (block_mean(a, x, y, z), block_mean(b, x, y, z));
                total += 1;
                same += ma
                    .iter()
                    .zip(mb)
                    .all(|(p, q)| (p - q).abs() <= BLOCK_TOLERANCE) as u32;
            }
            x += BLOCK;
        }
        y += BLOCK;
    }
    if total == 0 {
        1.0
    } else {
        same as f64 / total as f64
    }
}

/// Past the reference's background, read at its top left corner, by more than antialiasing noise. From the reference
/// alone: each picture reading its own would pass a whole page in the wrong colour.
fn has_content(p: &Picture, background: [u8; 3], x: u32, y: u32, z: &Zone) -> bool {
    let (x1, y1) = ((x + BLOCK).min(z.x + z.w), (y + BLOCK).min(z.y + z.h));
    (y..y1).any(|yy| {
        (x..x1).any(|xx| {
            let i = ((yy * p.width + xx) * 4) as usize;
            (0..3).any(|c| p.rgba[i + c].abs_diff(background[c]) > 2)
        })
    })
}

fn block_mean(p: &Picture, x: u32, y: u32, z: &Zone) -> [f64; 3] {
    let (x1, y1) = ((x + BLOCK).min(z.x + z.w), (y + BLOCK).min(z.y + z.h));
    let mut sum = [0f64; 3];
    for yy in y..y1 {
        for xx in x..x1 {
            let i = ((yy * p.width + xx) * 4) as usize;
            for c in 0..3 {
                sum[c] += p.rgba[i + c] as f64;
            }
        }
    }
    sum.map(|s| s / ((x1 - x) * (y1 - y)) as f64)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ui-spike")
        .to_path_buf()
}

pub fn write_png(path: &Path, p: &Picture) {
    fs::create_dir_all(path.parent().expect("a folder")).expect("mkdir");
    let mut enc = png::Encoder::new(
        BufWriter::new(File::create(path).expect("create")),
        p.width,
        p.height,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&p.rgba))
        .expect("png");
}

pub fn read_png(path: &Path) -> Picture {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("{}: {e}; write it with UPDATE_UI_REFS=1", path.display()));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("png header");
    let mut rgba = vec![0; reader.output_buffer_size().expect("png size")];
    let info = reader.next_frame(&mut rgba).expect("png frame");
    assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
    rgba.truncate(info.buffer_size());
    Picture {
        width: info.width,
        height: info.height,
        rgba,
    }
}

/// Compares one render to its reference, prints each zone's score, returns the zones under THRESHOLD as failures.
pub fn check(renderer: &str, name: &str, actual: &Picture) -> Vec<String> {
    let actual_path = root()
        .join("actual")
        .join(renderer)
        .join(format!("{name}.png"));
    write_png(&actual_path, actual);
    let ref_path = root().join("refs").join(format!("{name}.png"));
    if std::env::var("UPDATE_UI_REFS").as_deref() == Ok("1") && renderer == REF_OWNER {
        write_png(&ref_path, actual);
    }
    let reference = read_png(&ref_path);
    assert_eq!(
        (actual.width, actual.height),
        (reference.width, reference.height),
        "{name}: size differs from its reference"
    );
    let mut failed = vec![];
    for z in zones(actual.width, actual.height) {
        let s = score(actual, &reference, &z);
        println!("{renderer} {name} {}: {s:.3}", z.name);
        if s < THRESHOLD {
            failed.push(format!(
                "{name} {}: {s:.3} < {THRESHOLD} (see {})",
                z.name,
                actual_path.display()
            ));
        }
    }
    failed
}

const REF_OWNER: &str = "blitz";

/// Every reference name a renderer must draw: light and dark at each size, plus mid-list at the largest.
pub fn cases() -> Vec<Case> {
    let mut all = vec![];
    for dark in [false, true] {
        for (w, h) in crate::SIZES {
            all.push(Case {
                dark,
                w,
                h,
                scroll: 0.0,
            });
        }
        let (w, h) = crate::SIZES[2];
        all.push(Case {
            dark,
            w,
            h,
            scroll: crate::MID_SCROLL,
        });
    }
    all
}

pub struct Case {
    pub dark: bool,
    pub w: u32,
    pub h: u32,
    pub scroll: f64,
}

impl Case {
    pub fn name(&self) -> String {
        let theme = if self.dark { "dark" } else { "light" };
        let at = if self.scroll > 0.0 { "-mid" } else { "" };
        format!("{theme}-{}x{}{at}", self.w, self.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_all_in_the_wrong_colour_fails() {
        let reference = flat([255, 255, 255, 255]);
        let whole = Zone {
            name: "all",
            x: 0,
            y: 0,
            w: 64,
            h: 64,
        };
        assert_eq!(score(&flat([0, 0, 0, 255]), &reference, &whole), 0.0);
    }

    #[test]
    fn empty_background_does_not_dilute_a_missing_row() {
        let reference = flat([255, 255, 255, 255]);
        let mut with_row = flat([255, 255, 255, 255]);
        for x in 0..8 {
            with_row.rgba[((8 * 64 + x) * 4) as usize] = 0;
        }
        for y in 8..16 {
            for x in 0..8 {
                for c in 0..3 {
                    with_row.rgba[((y * 64 + x) * 4 + c) as usize] = 0;
                }
            }
        }
        let whole = Zone {
            name: "all",
            x: 0,
            y: 0,
            w: 64,
            h: 64,
        };
        assert_eq!(score(&reference, &with_row, &whole), 0.0);
    }

    fn flat(colour: [u8; 4]) -> Picture {
        Picture {
            width: 64,
            height: 64,
            rgba: colour.repeat(64 * 64),
        }
    }

    #[test]
    fn a_picture_matches_itself_and_a_wrong_colour_fails_only_its_zone() {
        let reference = flat([255, 255, 255, 255]);
        let whole = Zone {
            name: "all",
            x: 0,
            y: 0,
            w: 64,
            h: 64,
        };
        assert_eq!(score(&reference, &reference, &whole), 1.0);

        let mut changed = flat([255, 255, 255, 255]);
        for y in 0..32 {
            for x in 0..64 {
                changed.rgba[(y * 64 + x) * 4] = 0;
            }
        }
        let top = Zone {
            name: "top",
            x: 0,
            y: 0,
            w: 64,
            h: 32,
        };
        let bottom = Zone {
            name: "bottom",
            x: 0,
            y: 32,
            w: 64,
            h: 32,
        };
        assert_eq!(score(&changed, &reference, &top), 0.0);
        assert_eq!(score(&changed, &reference, &bottom), 1.0);
    }

    #[test]
    fn single_pixel_antialiasing_noise_stays_inside_the_tolerance() {
        let reference = flat([255, 255, 255, 255]);
        let mut noisy = flat([255, 255, 255, 255]);
        noisy.rgba[0] = 0;
        let whole = Zone {
            name: "all",
            x: 0,
            y: 0,
            w: 64,
            h: 64,
        };
        assert_eq!(score(&noisy, &reference, &whole), 1.0);
    }

    /// The check must stay sharp: a tolerance of 24 once passed a title moved 4 px on every screen.
    #[test]
    fn a_reference_with_its_text_moved_4_px_fails_its_zone() {
        let reference = read_png(&root().join("refs/light-800x600.png"));
        let mut moved = Picture {
            width: reference.width,
            height: reference.height,
            rgba: reference.rgba.clone(),
        };
        let row = (reference.width * 4) as usize;
        for line in moved.rgba.chunks_mut(row) {
            let x = 60 * 4;
            line.copy_within(x..row / 2, x + 4 * 4);
        }
        let zones = zones(reference.width, reference.height);
        let text = zones
            .iter()
            .find(|z| z.name == "text")
            .expect("the text zone");
        let s = score(&moved, &reference, text);
        assert!(s < THRESHOLD, "moved text scored {s}");
    }

    #[test]
    fn every_case_has_a_distinct_name() {
        let mut names: Vec<_> = cases().iter().map(Case::name).collect();
        assert_eq!(names.len(), 8);
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8);
    }
}
