//! JMAP over the caller's transport (RFC 8620): the session, a call's answers by id, and a query value as a form sends it. The
//! list and the thread both read through here, and nothing here reads either.
use serde_json::{Map, Value, json};

/// klar's own endpoint (klar company/mail/jmap.py); Stalwart answers it with a redirect to /jmap/session on the same host.
pub const SESSION: &str = "https://mail.klar.im/.well-known/jmap";

pub(crate) const MAIL: &str = "urn:ietf:params:jmap:mail";
pub(crate) const USING: [&str; 2] = ["urn:ietf:params:jmap:core", MAIL];

/// A GET of the URL with no body, else a POST of this JSON, as the account read: the answer's body, or why there is none.
pub type Http<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<String, String>;

/// The API's URL and the mail account's id, as the session names them.
pub(crate) fn session(http: Http) -> Result<(String, String), String> {
    let session = parsed(&http(SESSION, None)?)?;
    let api = session["apiUrl"]
        .as_str()
        .ok_or("jmap: the session names no API")?;
    let account = session["primaryAccounts"][MAIL]
        .as_str()
        .ok_or("jmap: the session names no mail account")?;
    Ok((api.to_string(), account.to_string()))
}

/// Each call's answer under its id, as one JSON value: a call the server left out reads as null, never a panic (a `Map`'s
/// index panics on a missing key; /code-review of rust-daily-today).
pub(crate) fn called(http: Http, api: &str, calls: Value) -> Result<Value, String> {
    answers(&http(
        api,
        Some(&json!({"using": USING, "methodCalls": calls}).to_string()),
    )?)
    .map(Value::Object)
}

fn parsed(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|_| "jmap: the server's answer is not JSON".to_string())
}

/// Each call's answer by its id, or the first method that answered `error`.
pub(crate) fn answers(body: &str) -> Result<Map<String, Value>, String> {
    let by_id = responses(body)?;
    match by_id.iter().find_map(|(id, r)| failed(id, r)) {
        Some(why) => Err(why),
        None => Ok(by_id),
    }
}

/// Each call's answer by its id, a method's `error` kept as `{"error": <it>}`: a send reads what its first call made.
pub(crate) fn responses(body: &str) -> Result<Map<String, Value>, String> {
    let body = parsed(body)?;
    let mut by_id = Map::new();
    for answer in body["methodResponses"]
        .as_array()
        .ok_or("jmap: the server's answer is not JMAP")?
    {
        let (name, result, id) = (
            &answer[0],
            &answer[1],
            answer[2].as_str().unwrap_or_default(),
        );
        let result = match name == "error" {
            true => json!({ "error": result }),
            false => result.clone(),
        };
        // the call's own answer comes first: a method may add implicit ones under its id (RFC 8621 7.5)
        by_id.entry(id).or_insert(result);
    }
    Ok(by_id)
}

/// The method `id` answered with an error, said.
pub(crate) fn failed(id: &str, result: &Value) -> Option<String> {
    let error = result.get("error")?;
    let why = format!(
        "{} {}",
        error["type"].as_str().unwrap_or_default(),
        error["description"].as_str().unwrap_or_default()
    );
    Some(format!("jmap: {id} failed: {}", why.trim()))
}

/// A query value: what a form would send, so the server's `param` reads it back.
pub fn encoded(value: &str) -> String {
    let keep = |b: u8| b.is_ascii_alphanumeric() || b"-._~@".contains(&b);
    value
        .bytes()
        .map(|b| {
            if keep(b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
