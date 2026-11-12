//! Jev (TypeSafe AI, the model `dash doc ready` asks), the binary's one client: the inbox's check pass and the RFC judge
//! (`rfc_judge`) both ask through it (task rust-task-rfc-judge moved it out of inbox.rs). Each question is a « noul », 0 to 1;
//! the API refuses a `text` one (400, tried 2026-11-11). Over curl, its key from bin/secret in curl's environment, never its
//! command line. CI and a test never pay (`paid_refused`).
use std::time::Duration;

pub const URL: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-1.13.0";
/// One call, at most.
pub const CALL: Duration = Duration::from_secs(60);
/// Its price in US dollars per input token, $0.042 a million (Klar's ledger, qualification/jev); its output is not priced.
pub const USD_PER_INPUT_TOKEN: f64 = 0.042 / 1e6;

/// Why no paid call runs here, when CI or nextest runs it.
pub fn paid_refused() -> Option<String> {
    let under = ["CI", "NEXTEST"]
        .into_iter()
        .find(|v| std::env::var_os(v).is_some())?;
    Some(format!(
        "{under} is set: a check never pays Jev or an agent there"
    ))
}

/// The FNV-1a hash of `s`: short, and the same in every build.
pub fn fnv(s: &str) -> u64 {
    (s.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Jev's key, read once a process: bin/secret is a Python start each.
pub fn key() -> Result<&'static String, String> {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(k) = KEY.get() {
        return Ok(k);
    }
    let bin = crate::pages::repo()
        .join("bin/secret")
        .display()
        .to_string();
    let key = dash_process::run(
        &bin,
        &["get", "typesafe", "TYPESAFE_API_KEY"],
        None,
        Duration::from_secs(20),
    )?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("no TYPESAFE_API_KEY in bin/secret's typesafe".into());
    }
    Ok(KEY.get_or_init(|| key))
}

/// One call: `questions` (a JSON object of questions) on `state`, its answer whole (`answers`, `usage`). `agent` names the
/// caller in its User-Agent. Refused under CI or a test before the key is read.
pub fn ask(
    questions: &serde_json::Value,
    state: serde_json::Value,
    agent: &str,
) -> Result<serde_json::Value, String> {
    if let Some(no) = paid_refused() {
        return Err(no);
    }
    let body =
        serde_json::json!({"state": state, "model": MODEL, "questions": questions}).to_string();
    let key = key()?;
    let args = curl_args(&body, agent);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (status, text, reason) =
        dash_process::answer("curl", &args, None, &[("KEY", key.as_str())], CALL)
            .map_err(String::from)?;
    let (got, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    if !status.success() || code != "200" {
        return Err(format!(
            "Jev answered {code}: {}{}",
            got.chars().take(200).collect::<String>(),
            reason.trim()
        ));
    }
    serde_json::from_str(got).map_err(|e| format!("Jev's answer: {e}"))
}

/// curl's words for one call: its own limit, `--max-time`, 5 s under `CALL`, the process's, so curl answers its timeout before
/// the process is ended; the key from its environment, never these words.
fn curl_args(body: &str, agent: &str) -> Vec<String> {
    let max = (CALL.as_secs() - 5).to_string();
    let words = [
        "-sS",
        "--max-time",
        &max,
        "--variable",
        "%KEY",
        "--expand-header",
        "Authorization: Bearer {{KEY}}",
        "-H",
        "Content-Type: application/json",
        "-A",
        agent,
        "--data-binary",
        body,
        "-w",
        "\n%{http_code}",
        URL,
    ];
    words.iter().map(|w| w.to_string()).collect()
}

/// Jev's « noul » for `question` in its answer.
pub fn noul(answer: &serde_json::Value, question: &str) -> Result<f64, String> {
    let n = answer["answers"][question]["noul"].as_f64();
    n.ok_or_else(|| format!("Jev gave no score for {question}"))
}

/// The input tokens an answer was billed for: 0 when it does not say.
pub fn input_tokens(answer: &serde_json::Value) -> u64 {
    answer["usage"]["input_tokens"].as_u64().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test never pays: the call is refused before the key is read, nextest setting NEXTEST.
    #[test]
    fn a_test_never_pays_jev() {
        let _budget = dash_meter::memory::TestBudget::new();
        let said = ask(&serde_json::json!({}), serde_json::json!({}), "test").expect_err("refused");
        assert!(said.contains("is set: a check never pays Jev"), "{said}");
    }

    /// Done-when 4, calls bounded: each call is bounded twice, curl's own `--max-time 55` and the process ended at `CALL`, 60 s;
    /// the key never in its words.
    #[test]
    fn each_call_is_bounded_by_curl_and_by_its_process() {
        let _budget = dash_meter::memory::TestBudget::new();
        let args = curl_args("{}", "site_perso/rfc-check");
        let at = args
            .iter()
            .position(|a| a == "--max-time")
            .expect("--max-time");
        assert_eq!((args[at + 1].as_str(), CALL.as_secs()), ("55", 60));
        assert!(
            args.iter()
                .all(|a| !a.contains("Bearer ") || a.contains("{{KEY}}")),
            "{args:?}"
        );
    }
}
