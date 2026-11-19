//! One binary per role, each held to its size (task rust-binary-split; @nha, 2026-11-14: « seems like a symptom of a bigger
//! issue imo », on dash ui's 170 MB debug binary and its __eh_frame past ld's 16 MB). crates/dash builds three ways, each a
//! cargo feature set of the one crate: the CLI with neither stack, the server with dioxus-liveview's, the window with
//! Dioxus Native's. Each role's dev build on the Mac is held in [`HELD`] to ceilings that only go down, its __eh_frame under
//! ld64's limit, past which ld warns that its compact unwind table cannot reach the DWARF unwind info.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// The ceilings, from rust/: `<role> <bytes> <__eh_frame bytes>` a line.
pub const HELD: &str = "binary-sizes-held.txt";

/// ld64's limit on __eh_frame: compact unwind stores an offset into it in 24 bits. Past it, every link of the binary prints
/// « __eh_frame section too large (max 16MB) to encode dwarf unwind offsets in compact unwind table ».
pub const EH_FRAME_MOST: u64 = 16 << 20;

/// Each role, the file rust/bin/dash links its build to under target/debug, and the cargo flags that build it.
pub const ROLES: [(&str, &str, &str); 3] = [
    ("cli", "dash-cli", "--no-default-features"),
    (
        "serve",
        "dash-serve",
        "--no-default-features --features server",
    ),
    (
        "window",
        "dash-window",
        "--no-default-features --features window",
    ),
];

/// Each role's ceilings, `(bytes, __eh_frame bytes)`, from the held file's text; a line that is not a role and two numbers
/// fails, `#` lines aside.
pub fn held(text: &str) -> Result<BTreeMap<String, (u64, u64)>, String> {
    let mut held = BTreeMap::new();
    for line in text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let words: Vec<&str> = line.split_whitespace().collect();
        let [role, bytes, eh] = words[..] else {
            return Err(format!(
                "{HELD}: « {line} »: a role, its bytes and its __eh_frame's"
            ));
        };
        let number = |w: &str| {
            w.parse::<u64>()
                .map_err(|e| format!("{HELD}: « {line} »: {e}"))
        };
        held.insert(role.to_string(), (number(bytes)?, number(eh)?));
    }
    Ok(held)
}

/// The ceilings that rose from `base`'s to `head`'s: they only go down. A role new at the head binds from then on.
pub fn rose(
    head: &BTreeMap<String, (u64, u64)>,
    base: &BTreeMap<String, (u64, u64)>,
) -> Vec<String> {
    head.iter()
        .filter_map(|(role, &(bytes, eh))| {
            let &(was, was_eh) = base.get(role)?;
            (bytes > was || eh > was_eh).then(|| {
                format!(
                    "{HELD}: {role} rose from {was} bytes, {was_eh} of __eh_frame, to {bytes} and {eh}: the ceilings only go \
                     down, so link less into the role (its features in crates/dash/Cargo.toml)"
                )
            })
        })
        .collect()
}

/// A role's binary, `(bytes, __eh_frame bytes)`, against its ceilings: the line to print, or why it fails.
pub fn verdict(
    role: &str,
    measured: (u64, u64),
    held: Option<(u64, u64)>,
) -> Result<String, String> {
    let (bytes, eh) = measured;
    let Some((most, most_eh)) = held else {
        return Err(format!(
            "{HELD}: no line for {role}: write « {role} {bytes} {eh} »"
        ));
    };
    let mut bad = vec![];
    if eh > EH_FRAME_MOST {
        bad.push(format!(
            "{role}: __eh_frame is {eh} bytes, past ld's {EH_FRAME_MOST}: ld warns at every link"
        ));
    }
    if bytes > most || eh > most_eh {
        bad.push(format!(
            "{role}: {bytes} bytes, {eh} of __eh_frame, past the {most} and {most_eh} held: link less into the role"
        ));
    }
    match bad.is_empty() {
        true => Ok(format!(
            "{role}: {bytes} bytes ({most} held), __eh_frame {eh} ({most_eh} held, ld's limit {EH_FRAME_MOST})"
        )),
        false => Err(bad.join("\n")),
    }
}

/// The size of __TEXT,__eh_frame in a thin 64-bit little-endian Mach-O's head (its header and load commands), 0 when it
/// has none; any other file fails.
/// ponytail: thin arm64 or x86_64 only, as cargo builds on this Mac; read a fat binary's slices if one is ever held.
pub fn eh_frame(head: &[u8]) -> Result<u64, String> {
    let u32_at = |at: usize| -> Result<u32, String> {
        let b = head.get(at..at + 4).ok_or("a Mach-O cut short")?;
        Ok(u32::from_le_bytes(b.try_into().unwrap_or_default()))
    };
    let u64_at = |at: usize| -> Result<u64, String> {
        let b = head.get(at..at + 8).ok_or("a Mach-O cut short")?;
        Ok(u64::from_le_bytes(b.try_into().unwrap_or_default()))
    };
    if u32_at(0)? != 0xfeed_facf {
        return Err("not a thin 64-bit Mach-O".into());
    }
    let named = |at: usize, name: &str| {
        head.get(at..at + 16)
            .is_some_and(|b| b.split(|&c| c == 0).next() == Some(name.as_bytes()))
    };
    let mut at = 32;
    for _ in 0..u32_at(16)? {
        let (cmd, size) = (u32_at(at)?, u32_at(at + 4)? as usize);
        // LC_SEGMENT_64: its sections, 80 bytes each, follow its 72
        if cmd == 0x19 && named(at + 8, "__TEXT") {
            for s in 0..u32_at(at + 64)? as usize {
                let section = at + 72 + s * 80;
                if named(section, "__eh_frame") {
                    return u64_at(section + 40);
                }
            }
        }
        at += size.max(8);
    }
    Ok(0)
}

/// A built binary's `(bytes, __eh_frame bytes)`, its head read alone.
pub fn measure(binary: &Path) -> Result<(u64, u64), String> {
    let mut file = std::fs::File::open(binary).map_err(|e| format!("{}: {e}", binary.display()))?;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let mut head = vec![0; 32];
    file.read_exact(&mut head)
        .map_err(|e| format!("{}: {e}", binary.display()))?;
    let commands = u32::from_le_bytes(head[20..24].try_into().unwrap_or_default()) as usize;
    // its magic first, then load commands inside the file: no header is read past its end (/code-review)
    if head[..4] != 0xfeed_facf_u32.to_le_bytes() || 32 + commands as u64 > bytes {
        return Err(format!("{}: not a thin 64-bit Mach-O", binary.display()));
    }
    head.resize(32 + commands, 0);
    file.read_exact(&mut head[32..])
        .map_err(|e| format!("{}: {e}", binary.display()))?;
    Ok((
        bytes,
        eh_frame(&head).map_err(|e| format!("{}: {e}", binary.display()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A Mach-O's head with one __TEXT segment holding __text and an __eh_frame of `eh` bytes, behind a segment of another name.
    fn mach_o(eh: u64) -> Vec<u8> {
        let mut b = vec![];
        let name = |b: &mut Vec<u8>, n: &str| {
            let mut field = [0u8; 16];
            field[..n.len()].copy_from_slice(n.as_bytes());
            b.extend(field);
        };
        b.extend(0xfeed_facf_u32.to_le_bytes());
        b.extend([0; 12]);
        b.extend(2u32.to_le_bytes());
        b.extend([0; 12]);
        // __PAGEZERO: a segment with no section
        b.extend(0x19u32.to_le_bytes());
        b.extend(72u32.to_le_bytes());
        name(&mut b, "__PAGEZERO");
        b.extend([0; 40]);
        b.extend(0u32.to_le_bytes());
        b.extend([0; 4]);
        b.extend(0x19u32.to_le_bytes());
        b.extend((72u32 + 160).to_le_bytes());
        name(&mut b, "__TEXT");
        b.extend([0; 40]);
        b.extend(2u32.to_le_bytes());
        b.extend([0; 4]);
        for (section, size) in [("__text", 999u64), ("__eh_frame", eh)] {
            name(&mut b, section);
            name(&mut b, "__TEXT");
            b.extend([0; 8]);
            b.extend(size.to_le_bytes());
            b.extend([0; 32]);
        }
        b
    }

    #[dash_test]
    fn the_eh_frame_is_read_from_the_text_segment_and_held_under_ld_s_limit() {
        assert_eq!(eh_frame(&mach_o(12_345)), Ok(12_345));
        assert_eq!(eh_frame(&mach_o(0)[..40]), Err("a Mach-O cut short".into()));
        assert_eq!(
            eh_frame(b"\x7fELF and more bytes"),
            Err("not a thin 64-bit Mach-O".into())
        );
        let held = held("# role bytes eh\ncli 100 10\nwindow 200 20\n").expect("held");
        assert_eq!(held["cli"], (100, 10));
        assert!(verdict("cli", (100, 10), held.get("cli").copied()).is_ok());
        let past = verdict("cli", (101, 10), held.get("cli").copied()).expect_err("a byte more");
        assert!(past.contains("past the 100 and 10 held"), "{past}");
        let warned = verdict("window", (1, EH_FRAME_MOST + 1), Some((u64::MAX, u64::MAX)))
            .expect_err("ld's limit");
        assert!(warned.contains("ld warns at every link"), "{warned}");
        assert!(
            verdict("serve", (1, 1), None)
                .expect_err("unheld")
                .contains("write « serve 1 1 »")
        );
        assert!(self::held("cli 1").is_err(), "a line with one number");
        let lower = self::held("cli 90 10\nwindow 200 21\nserve 5 5\n").expect("lower");
        let rose = rose(&lower, &held);
        assert_eq!(rose.len(), 1, "{rose:?}");
        assert!(
            rose[0].contains("window rose from 200 bytes, 20 of __eh_frame, to 200 and 21"),
            "{rose:?}"
        );
    }
}
