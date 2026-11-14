//! Task rust-tool-hostman through the real binary: `dash tools hostman` asks the hostman plugin's tool through the core's port,
//! the key read from `.secrets/hostman.env` and handed to curl in its environment, never its command line; the call lands in
//! the audit log with no line of the plugin's, and `dash audit hostman` sums it. curl is a stand-in answering Hostman's
//! recorded finances; no network is reached.
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::process::Command;

const DASH: &str = env!("CARGO_BIN_EXE_dash");

/// GET /api/v1/account/finances as Hostman answered it on 2026-11-14.
const FINANCES: &str = r#"{"finances":{"balance":30,"currency":"USD","discount_end_date_at":null,"discount_percent":0,"hourly_cost":0,"hourly_fee":0,"monthly_cost":0,"monthly_fee":0,"total_paid":30,"hours_left":null,"autopay_card_info":null,"total_balance":30},"response_id":"73086bef-7bdf-4738-af5a-12b184960b62"}"#;

/// curl as the tool runs it: the key in KEY alone, the URL Hostman's finances, the recorded answer and its code.
const CURL: &str = r#"#!/bin/sh
case "$*" in *stand-in-key-0000*) echo "the key is on the command line" >&2; exit 9;; esac
[ "$KEY" = "stand-in-key-0000" ] || { echo "no key in curl's environment" >&2; exit 8; }
for last; do :; done
[ "$last" = "https://hostman.com/api/v1/account/finances" ] || { echo "asked $last" >&2; exit 7; }
printf '%s\n%s' "$HOSTMAN_ANSWER" "$HOSTMAN_CODE"
"#;

/// bin/secret as /tools lists it: hostman's name and key name, then its value on a get.
const SECRET: &str = r#"#!/bin/sh
case "$1" in
  list) printf '[{"name":"hostman","kind":"API key","source":".secrets/hostman.env","state":"ok","keys":["HOSTMAN_API_KEY"]}]';;
  get) printf 'stand-in-key-0000';;
esac
"#;

fn dash(repo: &TestDir, path: &str, code: &str, args: &[&str]) -> std::process::Output {
    Command::new(DASH)
        .args(args)
        .env("HOME", repo.0.join("home"))
        .env("DASH_REPO", &repo.0)
        .env("PATH", path)
        .env("HOSTMAN_ANSWER", FINANCES)
        .env("HOSTMAN_CODE", code)
        .output()
        .expect("dash")
}

#[dash_test]
fn dash_tools_hostman_prints_the_credit_left_and_dash_audit_sums_its_calls() {
    let (repo, data, bin) = (
        TestDir::new("hostman-repo"),
        TestDir::new("hostman-data"),
        TestDir::new("hostman-bin"),
    );
    std::fs::create_dir_all(repo.0.join(".secrets")).expect("folder");
    std::fs::write(
        repo.0.join(".secrets/hostman.env"),
        "HOSTMAN_API_KEY=stand-in-key-0000\n",
    )
    .expect("key");
    std::os::unix::fs::symlink(dash_meter::stand_in(CURL), bin.0.join("curl")).expect("curl");
    let path = format!("{}:/usr/bin:/bin", bin.0.display());
    let dir = data.0.to_str().expect("a path");

    let out = dash(&repo, &path, "200", &["tools", "hostman", "--data", dir]);
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        printed.trim(),
        "hostman: 30.00 USD of credit left; the month's spend 0.00 USD (0.00 an hour, the servers running now); 0.00 USD \
         spent of 30.00 paid"
    );
    let refused = dash(&repo, &path, "401", &["tools", "hostman", "--data", dir]);
    assert!(!refused.status.success(), "{refused:?}");
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains("hostman answered 401") && !said.contains("stand-in-key"),
        "{said}"
    );

    // a name no tool holds says so (/code-review)
    let typo = dash(&repo, &path, "200", &["tools", "hostmn", "--data", dir]);
    assert!(
        String::from_utf8_lossy(&typo.stderr).contains("hostmn: no such tool"),
        "{typo:?}"
    );

    let sum = dash(&repo, &path, "200", &["audit", "hostman", "--data", dir]);
    let sum = String::from_utf8_lossy(&sum.stdout);
    assert!(
        sum.contains("hostman\t1\t1\t0\t") && sum.contains("hostman answered 401"),
        "one worked, one failed, its failure named: {sum}"
    );

    // /tools' text: hostman's card reads its usage through the same tool, so the live data's audit log keeps that call too
    std::fs::create_dir_all(repo.0.join("bin")).expect("bin");
    std::fs::create_dir_all(repo.0.join("home")).expect("home");
    std::os::unix::fs::symlink(dash_meter::stand_in(SECRET), repo.0.join("bin/secret"))
        .expect("secret");
    let live = repo.0.join("private/live/rust/data");
    std::fs::create_dir_all(&live).expect("live data");
    let tools = dash(&repo, &path, "200", &["access"]);
    let page = String::from_utf8_lossy(&tools.stdout);
    assert!(tools.status.success(), "{tools:?}");
    let card = page
        .lines()
        .find(|l| l.contains("hostman") && l.contains("credit left"));
    assert!(
        card.is_some_and(|l| l.contains("30.00 USD of credit left; the month's spend 0.00 USD")),
        "{page}"
    );
    let kept = dash(
        &repo,
        &path,
        "200",
        &["audit", "hostman", "--data", live.to_str().expect("a path")],
    );
    let kept = String::from_utf8_lossy(&kept.stdout);
    assert!(
        kept.contains("hostman\t1\t0\t0\t"),
        "/tools' read is kept: {kept}"
    );
}
