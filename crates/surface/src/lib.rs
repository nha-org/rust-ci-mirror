//! Adapter of section 3: what the app shows outside. For now the web renderer of an extension's UI tree and the MCP server
//! of the registry's tools. Hand-written JSON-RPC: rmcp would bring an async runtime for four methods.
use dash_core::registry::Registry;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Section 10's tree: a flat map of elements by id. Parsing it is the check: a bad type or prop is refused here.
#[derive(Deserialize)]
struct Tree {
    root: String,
    elements: HashMap<String, Node>,
}

#[derive(Deserialize)]
struct Node {
    #[serde(flatten)]
    element: Element,
    #[serde(default)]
    children: Vec<String>,
}

/// Section 12's catalog, a closed set: a new element type is a rebuild, a new tree of these is not. Props are meaning,
/// not markup: a plugin gives text, always escaped, and chooses no URL.
#[derive(Deserialize)]
#[serde(tag = "type", content = "props", rename_all = "kebab-case")]
enum Element {
    Stack,
    SearchBar { placeholder: String },
    Text { text: String },
}

/// How deep a tree may nest. The flat map parses at any depth, and a deep chain would overflow `draw`'s stack: an abort no
/// `catch_unwind` stops.
const DEPTH_LIMIT: usize = 64;

/// An element's HTML, its id the DOM id (section 10). `seen` refuses a cycle, which would never end, and an element with
/// two parents, which would repeat its id.
fn draw<'a>(
    tree: &'a Tree,
    id: &'a str,
    seen: &mut HashSet<&'a str>,
    depth: usize,
) -> Result<String, String> {
    if depth > DEPTH_LIMIT {
        return Err(format!("{id} is nested past {DEPTH_LIMIT} levels"));
    }
    if !seen.insert(id) {
        return Err(format!("{id} appears twice"));
    }
    let node = tree.elements.get(id).ok_or(format!("no element {id}"))?;
    let dom_id = escaped(id);
    let mut children = String::new();
    for child in &node.children {
        children += &draw(tree, child, seen, depth + 1)?;
    }
    Ok(match &node.element {
        Element::Stack => format!(r#"<div id="{dom_id}">{children}</div>"#),
        Element::SearchBar { placeholder } => format!(
            r#"<form id="{dom_id}" role="search" action="/search"><input type="search" name="q" placeholder="{}"><button>Search</button></form>"#,
            escaped(placeholder)
        ),
        Element::Text { text } => format!(r#"<p id="{dom_id}">{}</p>"#, escaped(text)),
    })
}

/// The HTML of a UI tree given as JSON text.
pub fn render(tree: &str) -> Result<String, String> {
    let tree: Tree = serde_json::from_str(tree).map_err(|e| e.to_string())?;
    draw(&tree, &tree.root, &mut HashSet::new(), 0)
}

fn tools_list(registry: &Registry) -> Value {
    let tools: Vec<Value> = registry
        .tools
        .iter()
        .map(|(name, _, tool)| {
            let schema: Value = serde_json::from_str(tool.input_schema())
                .unwrap_or_else(|_| json!({"type": "object"}));
            json!({"name": name, "description": tool.description(), "inputSchema": schema})
        })
        .collect();
    json!({"tools": tools})
}

/// A tool that fails answers `isError`, as MCP asks, so the agent reads why; an unknown tool is a protocol error.
fn tools_call(registry: &Registry, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "no tool name".to_string()))?;
    let tool = registry
        .tools
        .get(name)
        .ok_or((-32602, format!("no tool {name}")))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (text, is_error) = match tool.call(&arguments.to_string()) {
        Ok(text) => (text, false),
        Err(e) => (e, true),
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

fn answer(registry: &Registry, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "dash", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list(registry)),
        "tools/call" => tools_call(registry, params),
        other => Err((-32601, format!("no method {other}"))),
    }
}

fn failure(id: &Value, code: i64, text: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
}

/// The answer to one line, if it asks for one: a notification, or a client's response (we ask nothing), gets none.
fn respond(registry: &Registry, line: &[u8]) -> Option<Value> {
    let message: Value = match serde_json::from_slice(line) {
        Ok(message) => message,
        Err(e) => return Some(failure(&Value::Null, -32700, &e.to_string())),
    };
    if !message.is_object() {
        return Some(failure(&Value::Null, -32600, "a request is an object"));
    }
    let id = message.get("id")?;
    let method = message["method"].as_str()?;
    Some(match answer(registry, method, &message["params"]) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, text)) => failure(id, code, &text),
    })
}

/// MCP over stdio until its input ends: one JSON message a line. Lines are read as bytes: a stray byte is that line's parse
/// error, not the end of the server.
pub fn serve_mcp(
    registry: &Registry,
    input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    for line in input.split(b'\n') {
        let line = line?;
        if line.trim_ascii().is_empty() {
            continue;
        }
        if let Some(response) = respond(registry, &line) {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::{Contribution, Tool};
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_tree_renders_escaped_and_a_bad_one_is_refused() {
        let _budget = TestBudget::new();
        let html = render(
            r#"{"root":"page","elements":{
                "page":{"type":"stack","children":["bar","hint"]},
                "bar":{"type":"search-bar","props":{"placeholder":"a\"><script>"}},
                "hint":{"type":"text","props":{"text":"<b>&"}}}}"#,
        )
        .unwrap();
        assert_eq!(
            html,
            r#"<div id="page"><form id="bar" role="search" action="/search"><input type="search" name="q" placeholder="a&quot;&gt;&lt;script&gt;"><button>Search</button></form><p id="hint">&lt;b&gt;&amp;</p></div>"#
        );
        for (bad, says) in [
            (
                r#"{"root":"a","elements":{"a":{"type":"iframe"}}}"#,
                "iframe",
            ),
            (r#"{"root":"a","elements":{"a":{"type":"text"}}}"#, "props"),
            (
                r#"{"root":"a","elements":{"a":{"type":"text","props":{}}}}"#,
                "text",
            ),
            (r#"{"root":"none","elements":{}}"#, "no element none"),
            (
                r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#,
                "twice",
            ),
            (
                r#"{"root":"a","elements":{"a":{"type":"stack","children":["b","b"]},"b":{"type":"stack"}}}"#,
                "twice",
            ),
            ("not json", "expected"),
        ] {
            let e = render(bad)
                .err()
                .unwrap_or_else(|| panic!("{bad} rendered"));
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    #[test]
    fn a_tree_nested_too_deep_is_refused() {
        let _budget = TestBudget::new();
        let chain = |depth: usize| -> String {
            let elements: Vec<String> = (0..depth)
                .map(|i| format!(r#""{i}":{{"type":"stack","children":["{}"]}}"#, i + 1))
                .collect();
            format!(
                r#"{{"root":"0","elements":{{{},"{depth}":{{"type":"stack"}}}}}}"#,
                elements.join(",")
            )
        };
        assert!(render(&chain(DEPTH_LIMIT)).is_ok());
        for depth in [DEPTH_LIMIT + 1, 20_000] {
            let e = render(&chain(depth)).expect_err("too deep");
            assert!(e.contains("nested past"), "{e}");
        }
    }

    struct Fails;

    impl Tool for Fails {
        fn description(&self) -> &str {
            "It fails"
        }
        fn input_schema(&self) -> &str {
            r#"{"type":"object"}"#
        }
        fn call(&self, arguments: &str) -> Result<String, String> {
            Err(format!("no, given {arguments}"))
        }
    }

    #[test]
    fn mcp_answers_each_request_and_no_notification() {
        let _budget = TestBudget::new();
        let mut registry = Registry::default();
        let c = Contribution {
            tools: vec![("fails".into(), Box::new(Fails))],
            ..Default::default()
        };
        registry.add("t", c).unwrap();
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fails","arguments":{"a":1}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
            "not json",
            "[1]",
            r#"{"jsonrpc":"2.0","id":9,"result":{}}"#,
        ]
        .join("\n");
        let mut input = input.into_bytes();
        input.extend_from_slice(b"\n\xff\n");
        let mut output = vec![];
        serve_mcp(&registry, &input[..], &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 8, "a response gets no answer: {lines:?}");
        assert_eq!(lines[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(lines[1]["result"]["tools"][0]["name"], "fails");
        assert_eq!(
            lines[1]["result"]["tools"][0]["inputSchema"]["type"],
            "object"
        );
        assert_eq!(lines[2]["result"]["isError"], true);
        assert_eq!(
            lines[2]["result"]["content"][0]["text"],
            r#"no, given {"a":1}"#
        );
        assert_eq!(lines[3]["error"]["code"], -32602);
        assert_eq!(lines[4]["error"]["code"], -32601);
        assert_eq!(lines[5]["error"]["code"], -32700);
        assert_eq!(lines[6]["error"]["code"], -32600);
        assert_eq!(
            lines[7]["error"]["code"], -32700,
            "a stray byte, and it served on"
        );
    }
}
