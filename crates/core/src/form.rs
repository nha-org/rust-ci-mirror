//! A posted form's parts, as the serve reads them: the compass's comment, the extension's sending, /settings' restore.

/// One part of a multipart form: its field's name, its file's name when it is a file, its bytes.
pub struct Part<'a> {
    pub name: String,
    pub file: Option<String>,
    pub bytes: &'a [u8],
}

/// Each part of a `multipart/form-data` body, by the boundary its Content-Type names; none when it names no boundary. A part
/// cut short (no closing boundary) is left out: an upload that ended early lands nothing of it.
pub fn parts<'a>(body: &'a [u8], content_type: &str) -> Vec<Part<'a>> {
    let Some(boundary) = content_type.split("boundary=").nth(1) else {
        return vec![];
    };
    let boundary = boundary
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let (open, close) = (format!("--{boundary}"), format!("\r\n--{boundary}"));
    let Some(first) = find(body, open.as_bytes()).filter(|_| !boundary.is_empty()) else {
        return vec![];
    };
    let (mut rest, mut found) = (&body[first + open.len()..], vec![]);
    // after each boundary: « -- » ends the form, else a part's head, a blank line, its bytes up to the next boundary
    while let Some(after) = rest.strip_prefix(b"\r\n") {
        let Some(head_end) = find(after, b"\r\n\r\n") else {
            break;
        };
        let head = String::from_utf8_lossy(&after[..head_end]);
        let content = &after[head_end + 4..];
        let Some(end) = find(content, close.as_bytes()) else {
            break;
        };
        // Content-Disposition's `name="…"` and `filename="…"`, as a browser writes them
        let quoted = |key: &str| {
            let at = head.find(&format!("; {key}=\""))? + key.len() + 4;
            Some(head[at..].split('"').next()?.to_string())
        };
        found.push(Part {
            name: quoted("name").unwrap_or_default(),
            file: quoted("filename"),
            bytes: &content[..end],
        });
        rest = &content[end + close.len()..];
    }
    found
}

/// A form's fields and its files, each a name and its bytes.
pub type Form<'a> = (
    std::collections::BTreeMap<String, String>,
    Vec<(String, &'a [u8])>,
);

/// A multipart form's fields, the first of each name, and its files; a picker left empty sends a part with no name and no bytes.
pub fn form_parts<'a>(body: &'a [u8], kind: &str) -> Form<'a> {
    let (mut fields, mut files) = (std::collections::BTreeMap::new(), vec![]);
    for part in parts(body, kind) {
        match part.file {
            Some(name) if name.is_empty() && part.bytes.is_empty() => {}
            Some(name) => files.push((name, part.bytes)),
            None => {
                let value = String::from_utf8_lossy(part.bytes).into_owned();
                fields.entry(part.name).or_insert(value);
            }
        }
    }
    (fields, files)
}

/// `s` percent-encoded for a form, as bin/dash sends it.
pub fn encoded(s: &str) -> String {
    (s.bytes())
        .map(
            |b| match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                true => (b as char).to_string(),
                false => format!("%{b:02X}"),
            },
        )
        .collect()
}

/// A form bin/dash would send for `args` run in `cwd`.
pub fn form_of(cwd: &std::path::Path, args: &[&str]) -> String {
    let mut form = format!("cwd={}", encoded(&cwd.display().to_string()));
    for a in args {
        form.push_str(&format!("&args%5B%5D={}", encoded(a)));
    }
    form
}

/// `?view=`'s one of our three, in any case; any other value is the page's own.
pub fn view_named(query: &str) -> Option<&'static str> {
    match dash_sdk::param(query, "view")
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => Some("html"),
        Some("text") => Some("text"),
        Some("data") => Some("data"),
        _ => None,
    }
}

/// A page's form (the Rust tools' point 4): `?view=html|text` first, in any case, else HTML when the Accept header names it,
/// else the text. Any other view, an empty one included, leaves it to the header.
pub fn wants_html(accept: &str, query: &str) -> bool {
    let view = dash_sdk::param(query, "view").map(|v| v.trim().to_lowercase());
    match view.as_deref() {
        Some("html") => true,
        Some("text" | "data") => false,
        _ => accept.contains("text/html"),
    }
}
