//! A Steel script's entries. One engine boots once and each script runs in its own clone: a boot costs 33 MB and 118 ms in
//! release, another 13 MB and 93 ms, a clone 0.06 MB and 0.15 ms, and a clone sees no other script's definitions
//! (crates/dash/examples/extension_costs.rs).
//!
//! What this does not give (section 12, a test below holds each):
//! - Not a security boundary. Steel's sandbox stops files, sockets and dylibs, but a script can still spawn a process and
//!   read the environment, and `require` reads any path. The folder is the trust boundary, as a pod's manifest already
//!   runs any program.
//! - No memory cap: a runaway recursion reaches about 2 GB before Steel stops it. The deadline stops a loop, not this.
//! - An interrupt lands between instructions, so a script blocked in a native call (a sleep, waiting on a process it
//!   spawned) holds the thread past its deadline. Only moving calls off this thread would fix it.
//! - A wrong argument count on a script's own function can pass the load, and an extra argument can vanish with no error:
//!   only its tests catch these.
use dash_core::registry::{Contribution, Extract, Hit, Rerank, Tool};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Once, mpsc};
use std::time::Duration;
use steel::rvals::{FromSteelVal, SteelVal};
use steel::steel_vm::engine::Engine;

/// All a script gets from us. Its errors name what it was given, since a load names only the file.
const PRELUDE: &str = r#"
(define %registered '())
(define %tests '())
(define (%add! kind key value)
  (unless (string? key) (error kind "takes a string key, got" key))
  (set! %registered (cons (list kind key value) %registered)))
(define (%function! kind f)
  (unless (function? f) (error kind "takes a function, got" f)))
(define (register-extractor! key f)
  (%function! "register-extractor!" f)
  (%add! "extractor" key f))
(define (register-tool! name description schema f)
  (%function! "register-tool!" f)
  (%add! "tool" name (list description (value->jsexpr-string schema) f)))
(define (register-ui! slot tree)
  (%add! "ui" slot (value->jsexpr-string tree)))
(define (register-rerank! name f)
  (%function! "register-rerank!" f)
  (%add! "rerank" name f))
(define (test! name thunk)
  (%function! "test!" thunk)
  (set! %tests (cons (list name thunk) %tests)))
(define (check-equal? got want)
  (unless (equal? got want) (error "got" got "want" want)))
(define (%call-tool f arguments)
  (let ([answer (f (string->jsexpr arguments))])
    (if (string? answer) answer (value->jsexpr-string answer))))
(define (%rerank f query hits)
  (value->jsexpr-string (map (lambda (hit) (hash-ref hit 'id)) (f query (string->jsexpr hits)))))
"#;

/// A test that boots an engine. One is 33 MB of footprint on the Mac and two 47; Linux's resident reading also counts the
/// mapped debug binary, so the 50 MB default would pass here and fail on CI.
pub const STEEL_TEST_MB: f64 = 120.0;

/// The engine every script clones. Unset, Steel makes a folder in the user's home at its first use; set, it makes
/// `cached-modules` inside that folder. A home that cannot exist stops both: Steel ignores the failure to create it, and
/// a script then finds no shared modules. Steel reads it once, before any thread of ours starts.
pub fn base_engine() -> Engine {
    static STEEL_HOME: Once = Once::new();
    STEEL_HOME.call_once(|| unsafe { std::env::set_var("STEEL_HOME", "/nonexistent/steel-home") });
    let mut engine = Engine::new_sandboxed();
    engine.run(PRELUDE).expect("the prelude compiles");
    engine
}

/// One script's engine: not Send, so the registry lives on one thread (section 11's ceiling until a surface needs more).
struct Script {
    engine: RefCell<Engine>,
    limit: Duration,
    panicked: Cell<bool>,
}

impl Script {
    /// Each way into the engine: past the limit it is interrupted, a panic is an error. Steel's own `assert!` panics.
    fn guarded<T>(
        &self,
        run: impl FnOnce(&mut Engine) -> Result<T, steel::SteelErr>,
    ) -> Result<T, String> {
        if self.panicked.get() {
            return Err("this script panicked earlier and is off until the next start".into());
        }
        let mut engine = self.engine.borrow_mut();
        let controller = engine.get_thread_state_controller();
        let (done, finished) = mpsc::channel::<()>();
        let limit = self.limit;
        let watchdog = {
            let controller = controller.clone();
            std::thread::spawn(move || {
                if finished.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout) {
                    controller.interrupt();
                }
            })
        };
        let result = catch_unwind(AssertUnwindSafe(|| run(&mut engine)));
        drop(done);
        let _ = watchdog.join();
        controller.resume(); // Joined first: an interrupt that came as the call ended must not stop the next one.
        match result {
            Ok(answer) => answer.map_err(|e| e.to_string()),
            Err(_) => {
                self.panicked.set(true);
                Err("it panicked".into())
            }
        }
    }

    fn call(&self, name: &str, args: Vec<SteelVal>) -> Result<SteelVal, String> {
        self.guarded(|engine| {
            let f = engine.extract_value(name)?;
            engine.call_function_with_args(f, args)
        })
    }
}

fn text(s: &str) -> SteelVal {
    SteelVal::StringV(s.into())
}

fn string_of(value: &SteelVal) -> Result<String, String> {
    String::from_steelval(value).map_err(|_| format!("it answered {value:?}, not a string"))
}

fn items(value: &SteelVal) -> Result<Vec<SteelVal>, String> {
    Vec::<SteelVal>::from_steelval(value).map_err(|e| e.to_string())
}

struct ScriptExtract {
    script: Rc<Script>,
    f: SteelVal,
}

impl Extract for ScriptExtract {
    fn extract(&self, input: &[u8]) -> Result<String, String> {
        // ponytail: text only; binary input takes a compiled-in crate until a script needs a bytevector.
        let input = std::str::from_utf8(input).map_err(|_| "a script extracts text only")?;
        let f = self.f.clone();
        let answer = self
            .script
            .guarded(|engine| engine.call_function_with_args(f, vec![text(input)]))?;
        string_of(&answer)
    }
}

struct ScriptTool {
    script: Rc<Script>,
    description: String,
    schema: String,
    f: SteelVal,
}

impl Tool for ScriptTool {
    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> &str {
        &self.schema
    }

    fn call(&self, arguments: &str) -> Result<String, String> {
        let answer = self
            .script
            .call("%call-tool", vec![self.f.clone(), text(arguments)])?;
        string_of(&answer)
    }
}

struct ScriptRerank {
    script: Rc<Script>,
    f: SteelVal,
}

impl Rerank for ScriptRerank {
    fn rerank(&self, query: &str, hits: &[Hit]) -> Result<Vec<String>, String> {
        let hits: Vec<_> = hits
            .iter()
            .map(|h| json!({"id": h.id, "title": h.title, "body": h.body}))
            .collect();
        let args = vec![
            self.f.clone(),
            text(query),
            text(&serde_json::Value::from(hits).to_string()),
        ];
        let answer = string_of(&self.script.call("%rerank", args)?)?;
        serde_json::from_str(&answer).map_err(|_| format!("its ids are not strings: {answer}"))
    }
}

fn run_file(base: &Engine, path: &Path, limit: Duration) -> Result<Rc<Script>, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let script = Rc::new(Script {
        engine: RefCell::new(base.clone()),
        limit,
        panicked: Cell::new(false),
    });
    let path = path.to_path_buf();
    script.guarded(|engine| engine.compile_and_run_raw_program_with_path(source, path))?;
    Ok(script)
}

/// Runs the file in its own clone and answers what it registered, in the order it registered them.
pub fn load(base: &Engine, path: &Path, limit: Duration) -> Result<Contribution, String> {
    let script = run_file(base, path, limit)?;
    let registered = script.guarded(|engine| engine.extract_value("%registered"))?;
    let mut c = Contribution::default();
    for entry in items(&registered)?.iter().rev() {
        let [kind, key, value] = &items(entry)?[..] else {
            return Err("a registration is not (kind key value)".into());
        };
        let (kind, key) = (string_of(kind)?, string_of(key)?);
        let script = script.clone();
        match kind.as_str() {
            "extractor" => {
                let f = value.clone();
                c.extractors
                    .push((key, Box::new(ScriptExtract { script, f })));
            }
            "tool" => {
                let [description, schema, f] = &items(value)?[..] else {
                    return Err("a tool is not (description schema f)".into());
                };
                let schema = string_of(schema)?;
                // MCP's inputSchema is an object: a client that checks it refuses the whole of tools/list.
                if !serde_json::from_str::<Value>(&schema).is_ok_and(|s| s.is_object()) {
                    return Err(format!("tool {key}: its schema is not an object: {schema}"));
                }
                c.tools.push((
                    key,
                    Box::new(ScriptTool {
                        script,
                        description: string_of(description)?,
                        schema,
                        f: f.clone(),
                    }),
                ));
            }
            "ui" => c.ui.push((key, string_of(value)?.into())),
            "rerank" => {
                let f = value.clone();
                c.reranks.push((key, Box::new(ScriptRerank { script, f })));
            }
            other => return Err(format!("unknown registration {other}")),
        }
    }
    Ok(c)
}

/// A fresh run of the script, then each `test!` in it: section 12's « one nextest test per script ». A script with no test
/// fails: it carries its tests or it is not done. What a start would refuse is held by dash's `every_sample_loads_as_a_start_would`:
/// this crate cannot reach the renderer.
pub fn run_tests(path: &Path, limit: Duration) -> Result<Vec<String>, String> {
    let script = run_file(&base_engine(), path, limit)?;
    let tests = script.guarded(|engine| engine.extract_value("%tests"))?;
    let mut passed = vec![];
    for test in items(&tests)?.iter().rev() {
        let [name, thunk] = &items(test)?[..] else {
            return Err("a test is not (name thunk)".into());
        };
        let name = string_of(name)?;
        let thunk = thunk.clone();
        script
            .guarded(|engine| engine.call_function_with_args(thunk, vec![]))
            .map_err(|e| format!("{name}: {e}"))?;
        passed.push(name);
    }
    if passed.is_empty() {
        return Err("it has no test!".into());
    }
    Ok(passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use std::path::PathBuf;

    const LIMIT: Duration = Duration::from_secs(5);

    fn script_file(dir: &TestDir, source: &str) -> PathBuf {
        let path = dir.0.join("s.scm");
        std::fs::write(&path, source).expect("write");
        path
    }

    #[test]
    fn a_script_registers_each_kind_and_each_entry_calls_back_in() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-kinds");
        let base = base_engine();
        let path = script_file(
            &dir,
            r#"
            (register-extractor! "text/x" (lambda (s) (string-upcase s)))
            (register-tool! "echo" "Echoes" (hash "type" "object") (lambda (args) (hash-ref args 'word)))
            (register-tool! "list" "Lists" (hash "type" "object") (lambda (args) (list 1 "two")))
            (register-ui! "slot" (hash "root" "a" "elements" (hash)))
            (register-rerank! "reverse" (lambda (query hits) (reverse hits)))
            "#,
        );
        let c = load(&base, &path, LIMIT).expect("load");
        assert_eq!(c.extractors[0].1.extract(b"abc").unwrap(), "ABC");
        assert!(c.extractors[0].1.extract(b"\xff").is_err(), "text only");
        let (echo, list) = (&c.tools[0].1, &c.tools[1].1);
        assert_eq!(echo.description(), "Echoes");
        assert_eq!(echo.input_schema(), r#"{"type":"object"}"#);
        assert_eq!(echo.call(r#"{"word":"hey"}"#).unwrap(), "hey");
        assert_eq!(list.call("{}").unwrap(), r#"[1,"two"]"#);
        assert_eq!(&*c.ui[0].1, r#"{"elements":{},"root":"a"}"#);
        let hits: Vec<Hit> = ["a", "b"]
            .map(|id| Hit {
                id: id.into(),
                title: "t".into(),
                body: "b".into(),
            })
            .into();
        assert_eq!(c.reranks[0].1.rerank("q", &hits).unwrap(), ["b", "a"]);
    }

    #[test]
    fn two_scripts_share_no_definitions() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-isolated");
        let base = base_engine();
        let first = script_file(&dir, r#"(define secret "first")"#);
        load(&base, &first, LIMIT).expect("first");
        let second = script_file(&dir, "(register-ui! secret secret)");
        let e = load(&base, &second, LIMIT).err().expect("refused");
        assert!(e.contains("secret"), "{e}");
    }

    #[test]
    fn a_bad_script_is_refused_at_load() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-bad");
        let base = base_engine();
        for (source, says) in [
            ("(define (unbalanced", "Parse"),
            ("(define (f) (no-such-function 1))", "no-such-function"),
            (
                r#"(register-extractor! "k" "not a function")"#,
                "takes a function",
            ),
            (r#"(register-extractor! 1 (lambda (s) s))"#, "string key"),
            (
                r#"(register-tool! "t" "d" "not an object" (lambda (a) a))"#,
                "schema is not an object",
            ),
        ] {
            let path = script_file(&dir, source);
            let e = load(&base, &path, LIMIT).err().expect(source);
            assert!(e.contains(says), "{source}: {e}");
        }
    }

    /// Section 12 said an arity error fails at start. On a script's own function, Steel's optimizer decides: too few can be
    /// refused at load under a made-up name, the count can fail only when called, and an extra argument can vanish with no
    /// error at all. Only a script's tests catch the last two.
    #[test]
    fn a_wrong_argument_count_on_a_scripts_own_function_may_pass_the_load() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-arity");
        let base = base_engine();

        let path = script_file(
            &dir,
            r#"(define (one x) (string-append x "!"))
               (register-extractor! "k" (lambda (s) (one s s)))"#,
        );
        let c = load(&base, &path, LIMIT).expect("it loads");
        assert_eq!(c.extractors[0].1.extract(b"x").unwrap(), "x!", "dropped");

        let path = script_file(
            &dir,
            r#"(define (one x) (string-append x "!"))
               (register-extractor! "k" (lambda (s) (one)))"#,
        );
        let e = load(&base, &path, LIMIT).err().expect("too few, at load");
        assert!(e.contains("##x"), "under a made-up name: {e}");

        let path = script_file(
            &dir,
            r#"(define (two x) (list x x))
               (define (caller) (two 1 2))
               (test! "calls it wrong" (lambda () (caller)))"#,
        );
        assert!(load(&base, &path, LIMIT).is_ok(), "it loads");
        let e = run_tests(&path, LIMIT).expect_err("its test catches it");
        assert!(e.contains("calls it wrong") && e.contains("Arity"), "{e}");
    }

    #[test]
    fn a_runaway_call_is_stopped_at_its_limit_and_the_script_still_answers() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-loop");
        let base = base_engine();
        let path = script_file(
            &dir,
            r#"
            (define (spin) (spin))
            (register-extractor! "k" (lambda (s) (if (equal? s "spin") (spin) s)))
            "#,
        );
        let c = load(&base, &path, Duration::from_millis(300)).expect("load");
        let start = std::time::Instant::now();
        let e = c.extractors[0].1.extract(b"spin").expect_err("interrupted");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        assert!(e.contains("Interrupted"), "{e}");
        assert_eq!(c.extractors[0].1.extract(b"fine").unwrap(), "fine");

        let path = script_file(&dir, "(define (spin) (spin)) (spin)");
        let e = load(&base, &path, Duration::from_millis(300))
            .err()
            .expect("at load");
        assert!(e.contains("Interrupted"), "{e}");
    }

    /// Pins the ceiling in the module's header.
    #[test]
    fn a_blocking_native_call_outlasts_its_limit() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-blocked");
        let base = base_engine();
        let path = script_file(
            &dir,
            r#"(register-extractor! "k" (lambda (s) (time/sleep-ms 700) s))"#,
        );
        let c = load(&base, &path, Duration::from_millis(100)).expect("load");
        let start = std::time::Instant::now();
        let _ = c.extractors[0].1.extract(b"x");
        assert!(
            start.elapsed() >= Duration::from_millis(600),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_panic_in_steel_is_an_error_and_turns_that_script_off() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-panic");
        let base = base_engine();
        let path = script_file(
            &dir,
            r#"(register-extractor! "k" (lambda (s) (assert! #f)))"#,
        );
        let c = load(&base, &path, LIMIT).expect("load");
        let e = c.extractors[0].1.extract(b"x").expect_err("caught");
        assert!(e.contains("panicked"), "{e}");
        let e = c.extractors[0].1.extract(b"x").expect_err("off");
        assert!(e.contains("off until the next start"), "{e}");
    }

    #[test]
    fn a_failing_or_missing_test_fails_the_script() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-tests");
        let path = script_file(
            &dir,
            r#"(test! "good" (lambda () (check-equal? 1 1)))
               (test! "wrong" (lambda () (check-equal? (+ 1 1) 3)))"#,
        );
        let e = run_tests(&path, LIMIT).expect_err("wrong");
        assert!(e.contains("wrong") && e.contains("got 2 want 3"), "{e}");
        let path = script_file(&dir, "(define x 1)");
        assert!(
            run_tests(&path, LIMIT)
                .expect_err("none")
                .contains("no test!")
        );
        let path = script_file(&dir, r#"(test! "good" (lambda () (check-equal? 1 1)))"#);
        assert_eq!(run_tests(&path, LIMIT).unwrap(), ["good"]);
    }

    /// Pins the ceiling in the module's header: if Steel closes these, this fails and section 12 gets better news.
    #[test]
    fn steels_sandbox_stops_files_but_not_processes_or_the_environment() {
        let _budget = TestBudget::of(STEEL_TEST_MB);
        let dir = TestDir::new("script-sandbox");
        let base = base_engine();
        let path = script_file(
            &dir,
            r#"(test! "no file" (lambda () (open-input-file "/etc/hosts")))"#,
        );
        let e = run_tests(&path, LIMIT).expect_err("no file");
        assert!(e.contains("sandboxed"), "{e}");
        let path = script_file(
            &dir,
            r#"(register-ui! "child" (to-string (spawn-process (command "true" '()))))
               (register-ui! "env" (to-string (env-var "HOME")))"#,
        );
        let c = load(&base, &path, LIMIT).expect("both run");
        assert!(c.ui[0].1.contains("Ok"), "{}", c.ui[0].1);
        let home = std::env::var("HOME").expect("HOME");
        assert!(c.ui[1].1.contains(&home), "{}", c.ui[1].1);
    }
}
