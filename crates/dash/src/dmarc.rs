//! `dash dmarc <file>`: a domain's mail records, read for a lead (task dmarc-audit-leads, rfc-dmarc-outreach). One domain a
//! line in the file (`#` starts a comment); one line out per domain. A domain is a lead when it receives mail (an MX) and
//! its DMARC is missing or p=none, or its SPF is missing: Outlook.com refuses mail from a domain with no DMARC since
//! 2026-11 (HN 50416158). DNS goes through `dig +tcp`: his network rewrites plain DNS (CLAUDE.md).
use std::io::Write;
use std::process::Command;

/// One DNS question's answer: each record's text, or why there was none to read.
pub type Answer = Result<Vec<String>, String>;

/// What one domain's records say.
#[derive(Debug, PartialEq)]
pub struct Finding {
    pub domain: String,
    pub mx: bool,
    /// `missing`, `many` (two v=spf1 records void SPF, RFC 7208 4.5), or the `all` term: `-all`, `~all`, `?all`, `+all`.
    pub spf: String,
    /// `missing`, or the policy: `none`, `quarantine`, `reject`.
    pub dmarc: String,
    /// Reports asked (rua=): who reads them is the audit's first question.
    pub rua: bool,
}

impl Finding {
    pub fn lead(&self) -> bool {
        // an invalid p= is no policy; two SPF records or +all is no SPF (/code-review)
        self.mx
            && (!matches!(self.dmarc.as_str(), "quarantine" | "reject")
                || matches!(self.spf.as_str(), "missing" | "many" | "+all"))
    }
}

/// The value of `key=` in a `;`-separated tag list, as RFC 7489 writes DMARC's.
fn tag<'a>(record: &'a str, key: &str) -> Option<&'a str> {
    record.split(';').find_map(|t| {
        let (k, v) = t.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
    })
}

/// A domain's finding, its three questions asked of `lookup(name, type)`.
pub fn scan(domain: &str, lookup: &dyn Fn(&str, &str) -> Answer) -> Result<Finding, String> {
    // a null MX, `0 .`, says the domain takes no mail (RFC 7505)
    let mx = (lookup(domain, "MX")?.iter()).any(|r| r.split_whitespace().nth(1) != Some("."));
    let spfs: Vec<String> = lookup(domain, "TXT")?
        .into_iter()
        .filter(|t| (t.split_whitespace().next()).is_some_and(|v| v.eq_ignore_ascii_case("v=spf1")))
        .collect();
    let spf = match &spfs[..] {
        [] => "missing".to_string(),
        [one] => (one.split_whitespace().rev())
            .find(|t| {
                t.trim_start_matches(['+', '-', '~', '?'])
                    .eq_ignore_ascii_case("all")
            })
            .map(|t| {
                if t.eq_ignore_ascii_case("all") {
                    "+all".to_string()
                } else {
                    t.to_ascii_lowercase()
                }
            })
            // no all term: another domain's policy, or anything not listed is neutral, as ?all
            .unwrap_or_else(|| {
                let lower = one.to_ascii_lowercase();
                let redirect = lower.split_whitespace().any(|t| t.starts_with("redirect="));
                if redirect { "redirect" } else { "?all" }.to_string()
            }),
        _ => "many".to_string(),
    };
    let dmarcs: Vec<String> = lookup(&format!("_dmarc.{domain}"), "TXT")?
        .into_iter()
        .filter(|t| {
            (t.split(';').next()).is_some_and(|v| v.trim().eq_ignore_ascii_case("v=DMARC1"))
        })
        .collect();
    // RFC 7489 6.6.3: no record, or more than one, is no policy
    let (dmarc, rua) = match &dmarcs[..] {
        [one] => (
            tag(one, "p")
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "missing".into()),
            tag(one, "rua").is_some_and(|v| !v.is_empty()),
        ),
        _ => ("missing".to_string(), false),
    };
    Ok(Finding {
        domain: domain.to_string(),
        mx,
        spf,
        dmarc,
        rua,
    })
}

/// A TXT record's text: each quoted string, joined (a long SPF is split in 255-byte strings). Other types as they are.
// ponytail: a \" inside a TXT string splits wrongly; no SPF or DMARC record needs one
fn unquoted(rdata: &str) -> String {
    let parts: Vec<&str> = rdata.split('"').skip(1).step_by(2).collect();
    if parts.is_empty() {
        rdata.to_string()
    } else {
        parts.concat()
    }
}

/// dig's reply read: the last status decides (NOERROR or NXDOMAIN, else an error), then the answers of that type, a
/// CNAME on the way left out. `+short` was not enough: it prints nothing on SERVFAIL, read as « no record » (/code-review).
fn answers(text: &str, kind: &str) -> Answer {
    let status = (text.lines().rev())
        .find_map(|l| l.split("status: ").nth(1))
        .and_then(|rest| rest.split(',').next());
    match status {
        Some("NOERROR") | Some("NXDOMAIN") => {}
        Some(other) => return Err(other.to_string()),
        None => {
            let said = text.lines().find(|l| l.starts_with(";;"));
            return Err(said.unwrap_or("no reply").to_string());
        }
    }
    Ok((text.lines())
        .filter(|l| !l.starts_with(';') && !l.trim().is_empty())
        .filter_map(|l| {
            // name ttl class type rdata
            let mut rest = l;
            let mut fields = Vec::new();
            for _ in 0..4 {
                let word = rest.trim_start();
                let end = word.find(char::is_whitespace)?;
                fields.push(&word[..end]);
                rest = &word[end..];
            }
            fields[3]
                .eq_ignore_ascii_case(kind)
                .then(|| unquoted(rest.trim()))
        })
        .collect())
}

/// The live lookup: `dig +tcp`, bounded by its own timeout. A failure is said, never read as « no record ».
pub fn dig(name: &str, kind: &str) -> Answer {
    let out = Command::new("dig")
        .args([
            "+tcp",
            "+noall",
            "+comments",
            "+answer",
            "+time=5",
            "+tries=2",
            kind,
            name,
        ])
        .output()
        .map_err(|e| format!("dig: {e}"))?;
    answers(&String::from_utf8_lossy(&out.stdout), kind).map_err(|e| format!("{name} {kind}: {e}"))
}

/// The domains a file lists, lowercased, comments and blanks dropped.
pub fn domains(text: &str) -> Vec<String> {
    (text.lines())
        .map(|l| {
            l.split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches('.')
                .to_ascii_lowercase()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn line(f: &Finding) -> String {
    let yes = |b| if b { "yes" } else { "no" };
    format!(
        "{}\tmx={}\tspf={}\tdmarc={}\trua={}\tlead={}",
        f.domain,
        yes(f.mx),
        f.spf,
        f.dmarc,
        yes(f.rua),
        yes(f.lead())
    )
}

inventory::submit! { crate::usage::Command::usage("dash dmarc <file of domains, one a line>") }
/// The command: every domain's line, a lookup that failed said on its own line; exit 1 when any failed.
pub fn run(file: &str, mut out: std::fs::File) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("dmarc: {file}: {e}");
        std::process::exit(2);
    });
    let mut failed = false;
    // ponytail: one domain after another, three lookups each; threads when a list passes a few hundred
    for domain in domains(&text) {
        let said = match scan(&domain, &dig) {
            Ok(f) => line(&f),
            Err(e) => {
                failed = true;
                format!("{domain}\terror={e}")
            }
        };
        // a closed pipe (| head) ends the run quietly
        if writeln!(out, "{said}").is_err() {
            return;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Stand-in DNS: each (name, type) answers what the table says, anything else no record.
    fn table(
        rows: &'static [(&'static str, &'static str, &'static [&'static str])],
    ) -> impl Fn(&str, &str) -> Answer {
        move |name, kind| {
            Ok((rows.iter())
                .filter(|(n, k, _)| *n == name && *k == kind)
                .flat_map(|(_, _, a)| a.iter().map(|s| s.to_string()))
                .collect())
        }
    }

    /// The stand-in DNS every scan test reads: one domain per case.
    fn stand_in() -> impl Fn(&str, &str) -> Answer {
        table(&[
            ("bare.fr", "MX", &["10 mx.bare.fr."]),
            ("loose.fr", "MX", &["10 mx.loose.fr."]),
            (
                "loose.fr",
                "TXT",
                &[
                    "v=spf1 include:_spf.google.com ~all",
                    "google-site-verification=x",
                ],
            ),
            (
                "_dmarc.loose.fr",
                "TXT",
                &["v=DMARC1; p=none; rua=mailto:d@loose.fr"],
            ),
            ("tight.fr", "MX", &["10 mx.tight.fr."]),
            ("tight.fr", "TXT", &["v=spf1 mx -all"]),
            ("_dmarc.tight.fr", "TXT", &["v=DMARC1;p=REJECT"]),
            ("twice.fr", "MX", &["10 mx.twice.fr."]),
            ("twice.fr", "TXT", &["v=spf1 mx -all", "v=spf1 a ?all"]),
            (
                "_dmarc.twice.fr",
                "TXT",
                &["v=DMARC1; p=quarantine", "v=DMARC1; p=reject"],
            ),
            ("nomail.fr", "TXT", &["v=spf1 -all"]),
            ("null.fr", "MX", &["0 ."]),
            ("doublespf.fr", "MX", &["10 mx.d.fr."]),
            (
                "doublespf.fr",
                "TXT",
                &["v=spf1 mx -all", "v=spf1 a -all", "v=spf10 x"],
            ),
            ("_dmarc.doublespf.fr", "TXT", &["v=DMARC1; p=quarantine"]),
            ("redir.fr", "MX", &["10 mx.r.fr."]),
            ("redir.fr", "TXT", &["v=spf1 redirect=_spf.r.fr"]),
            ("open.fr", "MX", &["10 mx.o.fr."]),
            ("open.fr", "TXT", &["v=spf1 +all"]),
            ("_dmarc.open.fr", "TXT", &["v=DMARC1; p=reject"]),
            (
                "_dmarc.redir.fr",
                "TXT",
                &["v=DMARC1; p=reject", "v=DMARC1x; p=none"],
            ),
        ])
    }

    #[dash_test]
    fn dmarc_missing_none_or_doubled_is_a_lead_and_quarantine_or_reject_is_not() {
        let dns = stand_in();
        let dmarc = |d| {
            let f = scan(d, &dns).unwrap();
            (f.dmarc.clone(), f.rua, f.lead())
        };
        assert_eq!(dmarc("bare.fr"), ("missing".into(), false, true));
        assert_eq!(dmarc("loose.fr"), ("none".into(), true, true));
        assert_eq!(dmarc("tight.fr"), ("reject".into(), false, false));
        // two records are no policy (RFC 7489 6.6.3); v=DMARC1x is no record
        assert_eq!(dmarc("twice.fr"), ("missing".into(), false, true));
        assert_eq!(dmarc("redir.fr"), ("reject".into(), false, false));
        assert_eq!(
            line(&scan("loose.fr", &dns).unwrap()),
            "loose.fr\tmx=yes\tspf=~all\tdmarc=none\trua=yes\tlead=yes"
        );
    }

    #[dash_test]
    fn spf_missing_doubled_or_plus_all_is_a_lead_and_its_all_term_is_read() {
        let dns = stand_in();
        let spf = |d| scan(d, &dns).unwrap().spf;
        assert_eq!(spf("bare.fr"), "missing");
        assert_eq!(spf("loose.fr"), "~all");
        assert_eq!(spf("tight.fr"), "-all");
        assert_eq!(spf("redir.fr"), "redirect");
        // +all lets anyone send as the domain: a lead even under p=reject
        let open = scan("open.fr", &dns).unwrap();
        assert_eq!((open.spf.as_str(), open.lead()), ("+all", true));
        // two v=spf1 records void SPF (RFC 7208 4.5); v=spf10 is not one
        let double = scan("doublespf.fr", &dns).unwrap();
        assert_eq!(
            (double.spf.as_str(), double.dmarc.as_str(), double.lead()),
            ("many", "quarantine", true)
        );
    }

    #[dash_test]
    fn mx_missing_or_null_is_no_lead_whatever_the_rest() {
        let dns = stand_in();
        assert!(scan("bare.fr", &dns).unwrap().mx);
        let nomail = scan("nomail.fr", &dns).unwrap();
        assert_eq!(
            (nomail.mx, nomail.dmarc.as_str(), nomail.lead()),
            (false, "missing", false)
        );
        // a null MX, 0 ., takes no mail (RFC 7505)
        let null = scan("null.fr", &dns).unwrap();
        assert_eq!((null.mx, null.lead()), (false, false));
    }

    #[dash_test]
    fn a_failed_lookup_is_an_error_never_no_record() {
        let down = |_: &str, _: &str| -> Answer { Err("timed out".into()) };
        assert_eq!(scan("x.fr", &down), Err("timed out".into()));
    }

    #[dash_test]
    fn dig_text_and_files_read_as_written() {
        let reply = |status: &str, answer: &str| {
            format!(
                ";; ->>HEADER<<- opcode: QUERY, status: {status}, id: 1\n;; flags: qr rd ra;\n{answer}\n"
            )
        };
        let txt =
            "_dmarc.a.fr.\t300\tIN\tCNAME\tx.b.fr.\nx.b.fr.\t300\tIN\tTXT\t\"v=DMARC1; p=none\"";
        assert_eq!(
            answers(&reply("NOERROR", txt), "TXT"),
            Ok(vec!["v=DMARC1; p=none".to_string()])
        );
        assert_eq!(answers(&reply("NXDOMAIN", ""), "TXT"), Ok(vec![]));
        // SERVFAIL is an error, never an empty answer
        assert_eq!(
            answers(&reply("SERVFAIL", ""), "TXT"),
            Err("SERVFAIL".to_string())
        );
        let down = ";; communications error to 1.1.1.1#53: timed out\n";
        assert_eq!(answers(down, "MX"), Err(down.trim().to_string()));
        // a first try timed out, the retry answered: the answer stands
        let retried = format!("{down}{}", reply("NOERROR", "a.fr. 60 IN MX 10 mx.a.fr."));
        assert_eq!(answers(&retried, "MX"), Ok(vec!["10 mx.a.fr.".to_string()]));
        assert_eq!(
            unquoted("\"v=spf1 include:a \" \"-all\""),
            "v=spf1 include:a -all"
        );
        assert_eq!(unquoted("10 mx.a.fr."), "10 mx.a.fr.");
        assert_eq!(
            domains("# list\nA.fr.\n\nb.fr # shop\n"),
            vec!["a.fr", "b.fr"]
        );
    }
}
