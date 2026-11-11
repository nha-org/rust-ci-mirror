//! `dash doc extract` and `dash doc import`, the real binary (rfc-rust-unified section 20i, task rust-doc-core), on the
//! readers' fixtures in crates/ingest: no page's markup is written here (the gate's no-HTML rule for this crate).
use dash_meter::memory::TestBudget;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../ingest/tests/fixtures")
        .join(rel)
}

fn dash(args: &[&str], source: &str) -> (String, String) {
    let out = Command::new(DASH)
        .args(args)
        .arg(fixture(source))
        .output()
        .expect("dash runs");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    );
    assert!(out.status.success(), "dash {args:?} {source}: {stderr}");
    (stdout, stderr)
}

/// Done-when 2: one JSON item a line, title and text, as Flare's `extract` prints them, with its header and count on stderr.
#[test]
fn doc_extract_prints_the_items_as_flare_does() {
    let _budget = TestBudget::new();
    let (stdout, stderr) = dash(&["doc", "extract"], "supervisor-sandbox.html");
    let items: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert!(
        stderr.contains(r#"# header: {"format":"html"}"#),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("# done: {} items", items.len())),
        "{stderr}"
    );
    assert!(items.len() >= 5, "a chunk per section: {stdout}");
    let first = items[0]["fts"].as_str().unwrap_or_default();
    // its h1 is the heading path, so its description is not put first: web.clj does that for an item with no heading
    let h1 = "Claude is free inside its box. What leaves it goes through you.\n\nThe sandbox, the supervisor and the approval";
    assert!(
        first.starts_with(h1),
        "the heading path, then the body: {first}"
    );
    assert_eq!(
        items[1]["title"], "1The picture",
        "a section's item titled by its heading, as Jsoup reads it"
    );
    let titles: Vec<&str> = items.iter().filter_map(|i| i["title"].as_str()).collect();
    assert!(
        titles.iter().all(|t| !t.is_empty()),
        "every item titled: {titles:?}"
    );

    let (svg, _) = dash(&["doc", "extract"], "liteparse/sample.svg");
    let item: Value = serde_json::from_str(svg.trim()).expect("one item");
    assert!(
        item["fts"]
            .as_str()
            .is_some_and(|t| t.starts_with("Love comes through the stomach.")),
        "{item}"
    );
}

/// Done-when 3: the document whole, as canvist saves it: the inline SVG a diagram block, the PNG in it as its bytes.
#[test]
fn doc_import_prints_the_document_with_its_media() {
    let _budget = TestBudget::new();
    let (stdout, _) = dash(&["doc", "import"], "with-media/page.html");
    let saved: Value = serde_json::from_str(&stdout).expect("one JSON document");
    assert_eq!(saved["type"], "doc");
    let blocks = saved["content"].as_array().cloned().unwrap_or_default();
    let kind = |k: &str| {
        blocks
            .iter()
            .filter(|b| b["type"] == k)
            .cloned()
            .collect::<Vec<_>>()
    };
    let diagram = &kind("diagram")[0]["attrs"];
    assert!(
        diagram["svg"]
            .as_str()
            .is_some_and(|s| s.contains("viewBox=\"0 0 120 40\"")),
        "{diagram}"
    );
    let src = kind("image")[0]["attrs"]["src"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        src.starts_with("data:image/png;base64,iVBORw0KGgo"),
        "the PNG's own bytes: {src}"
    );
}

#[test]
fn doc_of_an_unknown_format_names_why() {
    let _budget = TestBudget::new();
    let out = Command::new(DASH)
        .args(["doc", "extract"])
        .arg(fixture("with-media/dot.png"))
        .output()
        .expect("dash runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && stderr.contains("no reader"),
        "{stderr}"
    );
}

/// Done-when 4: a PDF goes through the same two commands, unchanged since the HTML slice; its items carry their pages.
#[test]
fn doc_extract_and_import_take_a_pdf_as_they_are() {
    let _budget = TestBudget::new();
    let (stdout, stderr) = dash(&["doc", "extract"], "pdf/2206.01062.pdf");
    assert!(stderr.contains(r#"# header: {"format":"pdf"}"#), "{stderr}");
    let items: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    let first = items[0]["fts"].as_str().unwrap_or_default();
    assert!(
        first.starts_with("PDF · DocLayNet"),
        "the /Info preface first: {first}"
    );
    assert_eq!(items[0]["page"], 1);
    assert_eq!(
        items.last().map(|i| i["page"].clone()),
        Some(9.into()),
        "its last item on its last page"
    );
    let (saved, _) = dash(&["doc", "import"], "pdf/2206.01062.pdf");
    let saved: Value = serde_json::from_str(&saved).expect("one JSON document");
    assert!(
        saved["content"][0]["attrs"]["preface"]
            .as_str()
            .is_some_and(|p| p.contains("Birgit Pfitzmann"))
    );
}
