//! `oxplow extension new|check|test` (`.context/extensions.md` "The
//! SDK"). A thin argv shell over `oxplow_sdk`: the same `check` the
//! RPC/MCP `validate_extension` runs, printed as `file:line: what — fix`
//! lines so an agent editing an extension from a terminal gets the same
//! answer it would get through MCP. Exit 0 when clean, 1 with findings,
//! 2 on a usage error.

use std::io::Write;
use std::path::{Path, PathBuf};

use oxplow_app::extension_catalog::ExtensionCatalog;
use oxplow_app::extensions::EXTENSIONS_DIR;
use oxplow_sdk::{Format, Kind};

const USAGE: &str = "\
usage:
  oxplow extension new <lens|extension|provider|collector|command|effect|component|policy> <name> [--origin <ref>] [--capability <cap>] [--root <dir>]
      scaffold oxplow/extensions/<name>/ with a v2 manifest, an intent
      (--origin = the effort/thread ref that asked for it), one example
      and its fixture, and the kind's starter: a lens with a row action; a
      provider of --capability (work_items, the default, or effort_policy)
      with its contract's declarations, a stub program and test config; a Starlark
      collector with a model and a lens over it; a command composing core
      commands; an effect reacting to an event; a custom component with
      its lens and bundle; an effort policy script. Each checks clean
      and passes `test` as written
      (a provider once a real program replaces its stub)
  oxplow extension check <name|path> [--impact] [--against <rev>] [--json] [--root <dir>]
      load the extension and report every problem with file:line, dry-running
      its models, commands, lenses and advisories — against the project's
      database when it has been opened in oxplow (.oxplow/local.sqlite),
      else an empty one; command names against a throwaway oxplow.
      --impact also says what the working tree's version changes against
      git HEAD (or --against <rev>, which implies --impact): lenses' text,
      models and their rows, collectors' outputs, providers' grants —
      writing nothing
  oxplow extension test <name|path> [--bless] [--json] [--root <dir>]
      on a throwaway oxplow over a copy of the project's extensions: check,
      then each intent example's fixture (a lens's rows, a collector's
      entities, a command's composition), questions.yaml, and each declared
      provider: its handshake against its declarations, its test config,
      its examples, every message against the protocol's schemas, its
      golden transcript (--bless writes it) and its capability's
      conformance suite

<name|path> is an extension name under the project's oxplow/extensions/,
or the path to that folder. --root names the project (default: the
current directory, or the project the path sits in).
";

/// Run with `args` = everything after `extension`; returns the exit code.
pub fn run(args: &[String]) -> i32 {
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    run_to(args, &mut out, &mut err)
}

/// [`run`] writing to the given streams (what the tests call).
pub fn run_to(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match run_inner(args, out, err) {
        Ok(code) => code,
        Err(Failure::Usage(msg)) => {
            let _ = writeln!(err, "oxplow extension: {msg}\n\n{USAGE}");
            2
        }
        Err(Failure::Sdk(e)) => {
            let _ = writeln!(err, "oxplow extension: {e}");
            1
        }
    }
}

enum Failure {
    Usage(String),
    Sdk(oxplow_sdk::SdkError),
}

impl From<oxplow_sdk::SdkError> for Failure {
    fn from(e: oxplow_sdk::SdkError) -> Self {
        Failure::Sdk(e)
    }
}

struct Parsed {
    positional: Vec<String>,
    origin: Option<String>,
    /// `new provider --capability`: the capability it implements.
    capability: Option<String>,
    root: Option<PathBuf>,
    json: bool,
    bless: bool,
    /// `check --impact`: compare the working tree with `against`.
    impact: bool,
    against: Option<String>,
}

fn parse(args: &[String]) -> Result<Parsed, Failure> {
    let mut p = Parsed {
        positional: Vec::new(),
        origin: None,
        capability: None,
        root: None,
        json: false,
        bless: false,
        impact: false,
        against: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--origin" => {
                p.origin = Some(
                    it.next()
                        .ok_or_else(|| Failure::Usage("--origin needs a ref".into()))?
                        .clone(),
                )
            }
            "--capability" => {
                p.capability = Some(
                    it.next()
                        .ok_or_else(|| Failure::Usage("--capability needs a capability".into()))?
                        .clone(),
                )
            }
            "--root" => {
                p.root =
                    Some(PathBuf::from(it.next().ok_or_else(|| {
                        Failure::Usage("--root needs a directory".into())
                    })?))
            }
            "--json" => p.json = true,
            "--impact" => p.impact = true,
            "--against" => {
                p.against = Some(
                    it.next()
                        .ok_or_else(|| Failure::Usage("--against needs a git revision".into()))?
                        .clone(),
                )
            }
            "--bless" => p.bless = true,
            "-h" | "--help" | "help" => return Err(Failure::Usage("help".into())),
            flag if flag.starts_with('-') => {
                return Err(Failure::Usage(format!("unknown flag `{flag}`")))
            }
            _ => p.positional.push(a.clone()),
        }
    }
    Ok(p)
}

fn run_inner(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, Failure> {
    let p = parse(args)?;
    let mut pos = p.positional.iter();
    let Some(sub) = pos.next() else {
        return Err(Failure::Usage("missing subcommand".into()));
    };
    match sub.as_str() {
        "new" => {
            let kind = pos
                .next()
                .and_then(|k| Kind::parse(k))
                .ok_or_else(|| Failure::Usage(format!("new needs a kind: {}", Kind::NAMES)))?;
            let kind =
                match (kind, p.capability.as_deref()) {
                    (Kind::Provider { .. }, Some(capability)) => {
                        Kind::provider(capability).map_err(|e| Failure::Usage(e.to_string()))?
                    }
                    (_, Some(_)) => return Err(Failure::Usage(
                        "--capability is a provider's: `new provider <name> --capability <cap>`"
                            .into(),
                    )),
                    (kind, None) => kind,
                };
            let name = pos
                .next()
                .ok_or_else(|| Failure::Usage("new needs a name".into()))?;
            let root = p.root.clone().unwrap_or_else(cwd);
            let made = oxplow_sdk::scaffold(&root, kind, name, p.origin.as_deref())?;
            let _ = writeln!(out, "created {}", made.dir);
            for f in &made.files {
                let _ = writeln!(out, "  {f}");
            }
            let _ = writeln!(
                out,
                "next: fill in the TODOs in {}/extension.yaml, then `oxplow extension check {}`",
                made.dir, made.name
            );
            Ok(0)
        }
        "check" => {
            let target = pos
                .next()
                .ok_or_else(|| Failure::Usage("check needs an extension name or path".into()))?;
            let (root, name) = locate(p.root.as_deref(), target);
            let catalog = ExtensionCatalog::new();
            // Read-only, no migrations: this CLI may not be the app's
            // version, and it must never change the project's database.
            let db = oxplow_sdk::project_database(&root).and_then(|path| {
                match oxplow_db::Database::open_read_only(&path) {
                    Ok(db) => Some(db),
                    Err(e) => {
                        let _ = writeln!(
                            err,
                            "warning: could not open the project database ({}): {e}",
                            path.display()
                        );
                        None
                    }
                }
            });
            let layer = db.map(oxplow_app::sql_gateway::SqlGateway::new);
            // No running oxplow to ask which commands exist: the check
            // starts a throwaway one, and its impact review uses the same.
            let against = (p.impact || p.against.is_some())
                .then(|| p.against.clone().unwrap_or_else(|| "HEAD".into()));
            let report = block_on(oxplow_sdk::check(
                &root,
                &name,
                &catalog,
                layer.as_ref(),
                None,
                against.as_deref(),
            ))?;
            let format = if p.json { Format::Json } else { Format::Text };
            let _ = write!(out, "{}", oxplow_sdk::render_findings(&report, format));
            if p.json {
                let _ = writeln!(out);
            }
            Ok(if report.ok { 0 } else { 1 })
        }
        "test" => {
            let target = pos
                .next()
                .ok_or_else(|| Failure::Usage("test needs an extension name or path".into()))?;
            let (root, name) = locate(p.root.as_deref(), target);
            // A throwaway oxplow over a copy of the project's extensions:
            // the project's database is never opened.
            let report = block_on(oxplow_sdk::extension_test::test_extension(
                &root, &name, p.bless,
            ))?;
            let format = if p.json { Format::Json } else { Format::Text };
            let _ = write!(
                out,
                "{}",
                oxplow_sdk::extension_test::render(&report, format)
            );
            if p.json {
                let _ = writeln!(out);
            }
            Ok(if report.ok { 0 } else { 1 })
        }
        other => Err(Failure::Usage(format!("unknown subcommand `{other}`"))),
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `(project root, extension name)` for a bare name or a folder path. A
/// path `…/oxplow/extensions/<name>` names its own project; anything
/// else is a name under `--root` (or the current directory).
fn locate(root: Option<&Path>, target: &str) -> (PathBuf, String) {
    let name = oxplow_sdk::name_of(target).to_string();
    if let Some(root) = root {
        return (root.to_path_buf(), name);
    }
    let path = Path::new(target.trim_end_matches('/'));
    let under_extensions = path
        .parent()
        .and_then(|p| p.strip_prefix(p.parent()?.parent()?).ok())
        .is_some_and(|rel| rel == Path::new(EXTENSIONS_DIR));
    if under_extensions {
        if let Some(root) = path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let root = if root.as_os_str().is_empty() {
                cwd()
            } else {
                root.to_path_buf()
            };
            return (root, name);
        }
    }
    (cwd(), name)
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        reason = "a test seeds the database through its stores"
    )]

    use super::*;

    fn cli(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_to(&args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn new_lens_then_check_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let (code, out, err) = cli(&[
            "new",
            "lens",
            "demo",
            "--origin",
            "effort:eff1",
            "--root",
            root,
        ]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("created oxplow/extensions/demo\n"), "{out}");
        assert!(out.contains("lenses/demo.yaml"), "{out}");
        let folder = dir.path().join("oxplow/extensions/demo");
        let (code, out, err) = cli(&["check", folder.to_str().unwrap()]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("demo: 0 errors, 0 warnings"), "{out}");
        assert!(out.contains("dry-run on an empty database"), "{out}");
        // Same by name with --root, and as JSON.
        let (code, out, _) = cli(&["check", "demo", "--root", root, "--json"]);
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["extension"]["manifestVersion"], 2);
    }

    #[test]
    fn shared_with_an_experimental_kind_exits_one_with_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("oxplow/extensions/team");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("extension.yaml"),
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nui:\n  replacements: []\n",
        )
        .unwrap();
        let (code, out, _) = cli(&["check", folder.to_str().unwrap()]);
        assert_eq!(code, 1);
        assert!(
            out.contains(
                "error: oxplow/extensions/team/extension.yaml:9: `ui.replacements` is experimental"
            ),
            "{out}"
        );
        assert!(out.contains("team: 1 error, 0 warnings"), "{out}");
    }

    /// tsk865: there is no `migrate`: a manifest is v2 or doesn't load.
    #[test]
    fn usage_errors_exit_two() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        assert_eq!(cli(&[]).0, 2);
        assert_eq!(cli(&["frobnicate"]).0, 2);
        assert_eq!(cli(&["migrate", "old", "--root", root]).0, 2);
        let (code, _, err) = cli(&["new", "widget", "x", "--root", root]);
        assert_eq!(code, 2);
        assert!(err.contains("`component` or `policy`"), "{err}");
        let (code, _, err) = cli(&["check", "nope", "--root", root]);
        assert_eq!(code, 1);
        assert!(err.contains("no extension `nope`"), "{err}");
        let (code, _, err) = cli(&["new", "lens", "x", "--origin", "junk", "--root", root]);
        assert_eq!(code, 1);
        assert!(err.contains("not a canonical ref"), "{err}");
    }

    /// `new provider --capability` scaffolds a provider of that
    /// capability; one without a contract is refused, and the flag is a
    /// provider's alone.
    #[test]
    fn new_provider_takes_a_capability() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let (code, _, err) = cli(&[
            "new",
            "provider",
            "steward",
            "--capability",
            "effort_policy",
            "--root",
            root,
        ]);
        assert_eq!(code, 0, "{err}");
        let manifest =
            std::fs::read_to_string(dir.path().join("oxplow/extensions/steward/extension.yaml"))
                .unwrap();
        assert!(
            manifest.contains("capability: effort_policy\n"),
            "{manifest}"
        );
        let (code, _, err) = cli(&[
            "new",
            "provider",
            "git",
            "--capability",
            "vcs",
            "--root",
            root,
        ]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("`vcs`"), "{err}");
        let (code, _, err) = cli(&[
            "new",
            "lens",
            "demo",
            "--capability",
            "effort_policy",
            "--root",
            root,
        ]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("--capability"), "{err}");
    }

    /// `check` must never migrate or write the project's database, and an
    /// unreadable one is reported, not passed off as "none found".
    #[test]
    fn check_opens_the_project_database_read_only_and_reports_trouble() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        assert_eq!(cli(&["new", "lens", "demo", "--root", root]).0, 0);
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(dir.path().join(".oxplow/local.sqlite"), "not a database").unwrap();
        let (code, out, err) = cli(&["check", "demo", "--root", root]);
        assert_eq!(code, 0, "the manifest itself is fine: {out}{err}");
        assert!(err.contains("could not open the project database"), "{err}");
        assert!(out.contains("dry-run on an empty database"), "{out}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".oxplow/local.sqlite")).unwrap(),
            "not a database",
            "check wrote to the project's database"
        );
    }

    /// The fake provider's binary, built beside this test binary (the
    /// workspace build builds every crate's bins).
    fn fake_bin() -> PathBuf {
        let exe = std::env::current_exe().expect("test exe");
        let bin = exe
            .parent()
            .and_then(Path::parent)
            .expect("target/<profile>/deps")
            .join("oxplow-provider-fake");
        assert!(
            bin.is_file(),
            "{} is missing; build it with `cargo build -p oxplow-provider-fake`",
            bin.display()
        );
        bin
    }

    /// `extension test` against a fake service with nothing in it: its
    /// state outlives each process (what the kit's restart re-sends
    /// against), and the golden transcript records the refs it hands out.
    fn test_on_a_fresh_service(project: &Path, args: &[&str]) -> (i32, String, String) {
        let kept = project.join(".oxplow");
        let entries = match std::fs::read_dir(&kept) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            read => read.unwrap().collect(),
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("fake-state-"))
            {
                std::fs::remove_file(path).unwrap();
            }
        }
        cli(args)
    }

    /// `extension test` on a provider that is an effort policy: the kit
    /// runs the effort-policy suite against it, made the project's policy
    /// in a throwaway host — the fake in policy mode passes once blessed.
    #[test]
    fn extension_test_runs_the_effort_policy_suite_against_a_policy_provider() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let ext = dir.path().join("oxplow/extensions/steward");
        std::fs::create_dir_all(ext.join("bin")).unwrap();
        std::fs::create_dir_all(ext.join("fixtures")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: steward\ndescription: An effort policy.\nsharing: private\n\
             intent:\n  purpose: Open and close efforts as items start and finish.\n  examples:\n\
             \x20   - name: basic\n      input: { command: react, input: { event: { id: e1, type: thread.checkpoint, v: 1, seq: 1, source: system, subject: [], payload: {}, anchors: {} } } }\n\
             \x20     expect: nothing to do\n\
             providers:\n  - id: steward\n    capability: effort_policy\n    entry: bin/provider\n    declarations: provider.json\n    needs: [sql.read]\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("fixtures/basic.yaml"),
            "name: basic\ninput: { command: react, input: { event: { id: e1, type: thread.checkpoint, v: 1, seq: 1, source: system, subject: [], payload: {}, anchors: {} } } }\nexpect: { skip: $any }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("fixtures/provider-steward.yaml"),
            "config: { team: core }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("provider.json"),
            serde_json::to_string_pretty(&oxplow_provider_fake::policy_declarations()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            ext.join("bin/provider"),
            format!(
                "#!/bin/sh\nOXPLOW_FAKE_CAPABILITY=effort_policy OXPLOW_FAKE_STATE=\"{}/fake-state-$OXPLOW_PROVIDER_ID.json\" exec '{}' \"$@\"\n",
                dir.path().join(".oxplow").display(),
                fake_bin().display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(
                ext.join("bin/provider"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let (code, out, err) = cli(&["check", "steward", "--root", root]);
        assert_eq!(code, 0, "{out}{err}");
        let (code, out, err) =
            test_on_a_fresh_service(dir.path(), &["test", "steward", "--bless", "--root", root]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(
            out.contains("ran check, provider steward, effort_policy suite"),
            "{out}"
        );
        let (code, out, _) =
            test_on_a_fresh_service(dir.path(), &["test", "steward", "--root", root]);
        assert_eq!(code, 0, "a blessed transcript matches: {out}");
    }

    /// P5.D5's red: `extension test` on a scaffolded provider — the stub
    /// fails, the fake behind it passes once blessed, and a changed
    /// golden transcript fails naming the file and line.
    #[test]
    fn extension_test_blesses_a_provider_then_a_changed_transcript_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let (code, out, err) = cli(&["new", "provider", "fake", "--root", root]);
        assert_eq!(code, 0, "{err}");
        for f in [
            "provider.json",
            "bin/provider",
            "fixtures/provider-fake.yaml",
        ] {
            assert!(out.contains(f), "{out}");
        }
        assert_eq!(cli(&["check", "fake", "--root", root]).0, 0);
        let ext = dir.path().join("oxplow/extensions/fake");

        // The stub doesn't speak the protocol.
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 1, "{out}");
        assert!(
            out.contains("error: oxplow/extensions/fake/provider.json:1: initialize failed"),
            "{out}"
        );

        // The fake does: its declarations, a config its check accepts.
        std::fs::write(
            ext.join("bin/provider"),
            format!(
                "#!/bin/sh\nOXPLOW_FAKE_STATE=\"{}/fake-state-$OXPLOW_PROVIDER_ID.json\" exec '{}' \"$@\"\n",
                dir.path().join(".oxplow").display(),
                fake_bin().display()
            ),
        )
        .unwrap();
        std::fs::write(
            ext.join("provider.json"),
            serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
        )
        .unwrap();
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 1, "{out}");
        assert!(
            out.contains(
                "oxplow/extensions/fake/fixtures/provider-fake.yaml:1: check reports `/team`"
            ),
            "{out}"
        );
        std::fs::write(
            ext.join("fixtures/provider-fake.yaml"),
            "config: { team: core }\n",
        )
        .unwrap();
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 1, "{out}");
        assert!(
            out.contains("fixtures/transcripts/fake.jsonl:1: no golden transcript"),
            "{out}"
        );

        let (code, out, err) =
            test_on_a_fresh_service(dir.path(), &["test", "fake", "--bless", "--root", root]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(
            out.contains("blessed: oxplow/extensions/fake/fixtures/transcripts/fake.jsonl"),
            "{out}"
        );
        assert!(
            out.contains("ran check, provider fake, discover, read work_items, work_items suite"),
            "{out}"
        );
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 0, "a blessed transcript matches: {out}");

        // Its own questions: the skill file must name what they reach.
        std::fs::write(
            ext.join("questions.yaml"),
            "- question: File a ticket.\n  skill: README.md\n  reaches: { command: fake.create, input: { title: Hello } }\n",
        )
        .unwrap();
        std::fs::write(ext.join("README.md"), "# Fake tracker\n").unwrap();
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("oxplow/extensions/fake/questions.yaml: question 1 (\"File a ticket.\"): skill `README.md` never names `fake.create`"), "{out}");
        std::fs::write(
            ext.join("README.md"),
            "# Fake tracker\n\nFile one with `fake.create`.\n",
        )
        .unwrap();
        let (code, out, _) = test_on_a_fresh_service(dir.path(), &["test", "fake", "--root", root]);
        assert_eq!(code, 0, "{out}");
        assert!(
            out.contains(
                "ran check, provider fake, discover, read work_items, work_items suite, questions"
            ),
            "{out}"
        );

        // A changed golden is a diff at its line.
        let golden = ext.join("fixtures/transcripts/fake.jsonl");
        let text = std::fs::read_to_string(&golden).unwrap();
        let line = text.lines().position(|l| l.contains("\"First\"")).unwrap() + 1;
        std::fs::write(&golden, text.replace("\"First\"", "\"Second\"")).unwrap();
        let (code, out, _) =
            test_on_a_fresh_service(dir.path(), &["test", "fake", "--json", "--root", root]);
        assert_eq!(code, 1, "{out}");
        let report: serde_json::Value = serde_json::from_str(&out).unwrap();
        let errors = report["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let e = errors[0].as_str().unwrap();
        assert!(
            e.starts_with(&format!("oxplow/extensions/fake/fixtures/transcripts/fake.jsonl:{line}: the transcript differs at `/message/params/input/title`")),
            "{e}"
        );
        assert!(e.contains("--bless"), "{e}");
    }

    #[test]
    fn locate_reads_the_project_off_an_extension_path() {
        let (root, name) = locate(None, "/proj/oxplow/extensions/demo/");
        assert_eq!(root, PathBuf::from("/proj"));
        assert_eq!(name, "demo");
        let (root, name) = locate(Some(Path::new("/r")), "demo");
        assert_eq!(root, PathBuf::from("/r"));
        assert_eq!(name, "demo");
        assert_eq!(locate(None, "demo").1, "demo");
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// `check --impact` compares the working tree with `git:HEAD`
    /// (or `--against`): a lens and a model edited in the worktree read as
    /// changed, the model with its rows; `--json` carries the report; the
    /// project's database isn't touched.
    #[test]
    fn check_impact_compares_the_worktree_with_head_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ext = root.join("oxplow/extensions/acme");
        let w = |rel: &str, body: &str| {
            let p = ext.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        w(
            "extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: Count.\n  origin: thread:thr1\n  examples: []\nmodels:\n  - name: x\n    version: 1\n    description: X.\n    columns:\n      - { name: n, type: \"\", doc: N. }\n",
        );
        w("models/x.sql", "SELECT 1 AS n\n");
        w(
            "lenses/count.yaml",
            "title: Count\nquery: SELECT n FROM v_acme_x\nviz: number\n",
        );
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "init"]);
        // A project database the check may read but must not change.
        let db_path = root.join(".oxplow/local.sqlite");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        drop(oxplow_db::Database::open(&db_path).unwrap());
        let db_before = std::fs::read(&db_path).unwrap();

        w("models/x.sql", "SELECT 2 AS n\n");
        let root_s = root.to_str().unwrap();
        let (code, out, err) = cli(&["check", "acme", "--root", root_s, "--impact"]);
        assert_eq!(code, 0, "{out}{err}");
        assert!(out.contains("impact against HEAD:"), "{out}");
        assert!(out.contains("Lens acme/count: changed"), "{out}");
        assert!(out.contains("Model v_acme_x rows: 1 → 1"), "{out}");

        let (code, out, _) = cli(&["check", "acme", "--root", root_s, "--impact", "--json"]);
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(v["impact"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "Lens acme/count: changed"));

        assert_eq!(
            std::fs::read(&db_path).unwrap(),
            db_before,
            "the database is untouched"
        );
    }
}
