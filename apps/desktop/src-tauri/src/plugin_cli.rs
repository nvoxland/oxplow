//! `oxplow plugin new|check|migrate` (`.context/extensions.md` "The
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
  oxplow plugin new <lens|extension> <name> [--origin <ref>] [--root <dir>]
      scaffold oxplow/extensions/<name>/ with a v2 manifest, an intent
      (--origin = the effort/thread ref that asked for it), one example
      and fixture — and, for a lens, one starter lens
  oxplow plugin check <name|path> [--json] [--root <dir>]
      load the extension and report every problem with file:line; when the
      project has been opened in oxplow (.oxplow/local.sqlite exists) every
      lens and advisory is also dry-run against its database
  oxplow plugin migrate <name|path> [--root <dir>]
      rewrite a v1 extension.yaml as v2 in place (idempotent)

<name|path> is an extension name under the project's oxplow/extensions/,
or the path to that folder. --root names the project (default: the
current directory, or the project the path sits in).
";

/// Run with `args` = everything after `plugin`; returns the exit code.
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
            let _ = writeln!(err, "oxplow plugin: {msg}\n\n{USAGE}");
            2
        }
        Err(Failure::Sdk(e)) => {
            let _ = writeln!(err, "oxplow plugin: {e}");
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
    root: Option<PathBuf>,
    json: bool,
}

fn parse(args: &[String]) -> Result<Parsed, Failure> {
    let mut p = Parsed {
        positional: Vec::new(),
        origin: None,
        root: None,
        json: false,
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
            "--root" => {
                p.root =
                    Some(PathBuf::from(it.next().ok_or_else(|| {
                        Failure::Usage("--root needs a directory".into())
                    })?))
            }
            "--json" => p.json = true,
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
                .ok_or_else(|| Failure::Usage("new needs a kind: `lens` or `extension`".into()))?;
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
                "next: fill in the TODOs in {}/extension.yaml, then `oxplow plugin check {}`",
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
            let layer = db.map(oxplow_db::SemanticLayer::new);
            let report = block_on(oxplow_sdk::check(&root, &name, &catalog, layer.as_ref()))?;
            let format = if p.json { Format::Json } else { Format::Text };
            let _ = write!(out, "{}", oxplow_sdk::render_findings(&report, format));
            if p.json {
                let _ = writeln!(out);
            }
            Ok(if report.ok { 0 } else { 1 })
        }
        "migrate" => {
            let target = pos
                .next()
                .ok_or_else(|| Failure::Usage("migrate needs an extension name or path".into()))?;
            let (root, name) = locate(p.root.as_deref(), target);
            let done = oxplow_sdk::migrate(&root, &name)?;
            let _ = writeln!(
                out,
                "{}: {}",
                done.path,
                if done.changed {
                    "migrated to manifest v2; fill in intent.origin and intent.examples"
                } else {
                    "already v2, nothing to do"
                }
            );
            Ok(0)
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
        assert!(out.contains("lens SQL was not dry-run"), "{out}");
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
            "manifest: 2\nname: team\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nref_kinds:\n  - kind: ticket\n",
        )
        .unwrap();
        let (code, out, _) = cli(&["check", folder.to_str().unwrap()]);
        assert_eq!(code, 1);
        assert!(
            out.contains(
                "error: oxplow/extensions/team/extension.yaml:8: `ref_kinds` is experimental"
            ),
            "{out}"
        );
        assert!(out.contains("team: 1 error, 0 warnings"), "{out}");
    }

    #[test]
    fn migrate_rewrites_once_and_usage_errors_exit_two() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        let folder = dir.path().join("oxplow/extensions/old");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("extension.yaml"),
            "name: old\ndescription: Old\n",
        )
        .unwrap();
        let (code, out, _) = cli(&["migrate", "old", "--root", root]);
        assert_eq!(code, 0);
        assert!(out.contains("migrated to manifest v2"), "{out}");
        let (code, out, _) = cli(&["migrate", folder.to_str().unwrap()]);
        assert_eq!(code, 0);
        assert!(out.contains("already v2"), "{out}");
        let (code, _, err) = cli(&["check", "old", "--root", root]);
        assert_eq!(code, 0, "{err}");

        assert_eq!(cli(&[]).0, 2);
        assert_eq!(cli(&["frobnicate"]).0, 2);
        let (code, _, err) = cli(&["new", "widget", "x", "--root", root]);
        assert_eq!(code, 2);
        assert!(err.contains("`lens` or `extension`"), "{err}");
        let (code, _, err) = cli(&["check", "nope", "--root", root]);
        assert_eq!(code, 1);
        assert!(err.contains("no extension `nope`"), "{err}");
        let (code, _, err) = cli(&["new", "lens", "x", "--origin", "junk", "--root", root]);
        assert_eq!(code, 1);
        assert!(err.contains("not a canonical ref"), "{err}");
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
        assert!(out.contains("lens SQL was not dry-run"), "{out}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".oxplow/local.sqlite")).unwrap(),
            "not a database",
            "check wrote to the project's database"
        );
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
}
