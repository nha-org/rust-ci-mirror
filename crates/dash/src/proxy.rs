//! `dash mcp-proxy --cache <file> -- <server> [<argument>...]`: an MCP server behind its last tool list (rust/CLAUDE.md rule 5:
//! a tool list is never empty while its server refills; the bb mcp-proxy's LinkedIn list read [] after a rev bump, 2026-11-08).
//! It answers initialize and tools/list from `cache` with no server running, starts the server on the first other request,
//! and keeps the list the server gives at each start, unless that list is empty or unread: then the last one stays. The thin
//! slice of task rust-old-defects; task rust-mcp-proxy-cache puts it in front of LinkedIn.
//! ponytail: tools only. Its initialize is its own (no server instructions, resources or prompts), a list is refreshed only
//! when a call starts the server (no fingerprint of the server's sources, the bb proxy's), a server's own notifications and
//! requests are dropped, and there is no idle stop; each lands when a server that needs it moves here (/code-review of
//! rust-old-defects).
use dash_ext::pod::Running;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// The server's start with its list, then each request, at most this long each.
const LIMIT: Duration = Duration::from_secs(60);

/// The cached list: `[]` when there is none yet or it does not read.
fn last(cache: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(cache).unwrap_or_default();
    let list: Value = serde_json::from_str(&text).unwrap_or_default();
    list["tools"].as_array().cloned().unwrap_or_default()
}

/// The server's list into `cache`, written whole through this process's own temporary file, unless it is empty or unread.
/// What the cache holds after.
fn refill(server: &mut Running, cache: &Path, deadline: Instant) -> Vec<Value> {
    let fresh = (server.request("tools/list", json!({}), deadline).ok())
        .and_then(|r| r["tools"].as_array().cloned())
        .unwrap_or_default();
    if fresh.is_empty() {
        return last(cache);
    }
    // its own name: two sessions' proxies on one cache never write one temporary file
    let part = cache.with_extension(format!("part.{}", std::process::id()));
    let wrote = std::fs::write(&part, json!({"tools": fresh}).to_string())
        .and_then(|()| std::fs::rename(&part, cache));
    if let Err(e) = wrote {
        eprintln!("mcp-proxy: {}: {e}", cache.display());
    }
    fresh
}

/// The server running, started with its list refilled if it was not; the list when it was started now.
fn started<'a>(
    server: &'a mut Option<Running>,
    command: &[String],
    cache: &Path,
) -> Result<(&'a mut Running, Option<Vec<Value>>), String> {
    let mut tools = None;
    if server.is_none() {
        let folder = std::env::current_dir().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + LIMIT;
        let mut fresh = Running::start(command, &folder, deadline)?;
        tools = Some(refill(&mut fresh, cache, deadline));
        *server = Some(fresh);
    }
    let running = server.as_mut().ok_or("not started")?;
    Ok((running, tools))
}

/// The answer to one request, or why no answer came from the server.
fn answer(
    server: &mut Option<Running>,
    command: &[String],
    cache: &Path,
    id: &Value,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    let result = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    Ok(match method {
        "initialize" => result(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "dash-mcp-proxy", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => result(json!({})),
        "tools/list" => match last(cache) {
            tools if !tools.is_empty() => result(json!({ "tools": tools })),
            _ => {
                let (running, tools) = started(server, command, cache)?;
                let tools = tools.unwrap_or_else(|| refill(running, cache, Instant::now() + LIMIT));
                result(json!({ "tools": tools }))
            }
        },
        _ => {
            let (running, _) = started(server, command, cache)?;
            let mut answer = running.ask(method, params.clone(), Instant::now() + LIMIT)?;
            // the server's own result or error, under the client's id
            answer["id"] = id.clone();
            answer
        }
    })
}

/// Serves `input` until it ends, one JSON-RPC message a line, each answer a line on `out`.
pub fn serve(
    cache: &Path,
    command: &[String],
    input: impl BufRead,
    mut out: impl Write,
) -> Result<(), String> {
    let mut server: Option<Running> = None;
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => {
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})
            }
            // a notification or a response needs no answer, and the server got its own initialized at its start
            Ok(m) if m.get("id").is_none() || m.get("method").is_none() => continue,
            Ok(m) => {
                let (id, method) = (&m["id"], m["method"].as_str().unwrap_or_default());
                match answer(&mut server, command, cache, id, method, &m["params"]) {
                    Ok(reply) => reply,
                    // no answer came: the server starts again on the next request, so no late answer is read as that one's
                    Err(e) => {
                        server = None;
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": e}})
                    }
                }
            }
        };
        writeln!(out, "{reply}")
            .and_then(|()| out.flush())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
