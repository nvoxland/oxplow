//! `oxplow extension grammar build <name> [--language <id>]`: compile a
//! language's tree-sitter grammar to WebAssembly, for a language declared
//! `grammar: { wasm: grammars/<x>.wasm, source: { repository, rev, path? } }`
//! (`.context/language-support.md`).
//!
//! This is the author's tool, run by hand on the author's machine; oxplow
//! itself never fetches or compiles a grammar. It needs `git` and network
//! access for the grammar and, the first time, for wasi-sdk (or
//! `WASI_SDK_PATH` names one). It compiles as Zed does
//! (`crates/extension/src/extension_builder.rs`): wasi-sdk's `clang` with
//! `-fPIC -shared -Os -Wl,--export=tree_sitter_<name>`, the grammar's
//! `src/parser.c` and its `src/scanner.c` when there is one.

use std::path::{Path, PathBuf};
use std::process::Command;

use oxplow_code_metrics::{GrammarSource, LanguageDecl};

/// The wasi-sdk release the grammars are built with (Zed's).
pub const WASI_SDK_VERSION: &str = "34";

/// One language to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildJob {
    pub language: String,
    /// The file the manifest's `wasm:` names, in the extension.
    pub wasm: PathBuf,
    pub source: GrammarSource,
}

/// The languages of the extension at `dir` that declare `wasm` and `source`
/// (only `language` when given), read from its manifest. A language with a
/// `wasm` and no `source` has nothing to build from, and a `source` with no
/// `wasm` nowhere to write: each is said, when asked for by name.
pub fn jobs(dir: &Path, language: Option<&str>) -> Result<Vec<BuildJob>, String> {
    let manifest = std::fs::read_to_string(dir.join("extension.yaml"))
        .map_err(|e| format!("{}: {e}", dir.join("extension.yaml").display()))?;
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&manifest).map_err(|e| format!("extension.yaml: {e}"))?;
    let decls: Vec<LanguageDecl> = match doc.get("languages") {
        Some(list) => serde_yaml::from_value(list.clone())
            .map_err(|e| format!("extension.yaml `languages:`: {e}"))?,
        None => Vec::new(),
    };
    if let Some(want) = language {
        if !decls.iter().any(|d| d.id == want) {
            return Err(format!(
                "the extension declares no language `{want}` (it has: {})",
                decls
                    .iter()
                    .map(|d| d.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let mut out = Vec::new();
    for decl in decls
        .into_iter()
        .filter(|d| language.is_none_or(|want| d.id == want))
    {
        let named = language.is_some();
        match decl.grammar {
            Some(g) => match (g.wasm, g.source) {
                (Some(wasm), Some(source)) => out.push(BuildJob {
                    language: decl.id,
                    wasm: dir.join(wasm),
                    source,
                }),
                (Some(_), None) if named => {
                    return Err(format!(
                        "language `{}` declares `grammar.wasm` but no `grammar.source`: say where to build it from (`source: {{ repository, rev, path? }}`)",
                        decl.id
                    ))
                }
                (None, Some(_)) if named => {
                    return Err(format!(
                        "language `{}` declares `grammar.source` but no `grammar.wasm`: name the file to write (`wasm: grammars/<x>.wasm`)",
                        decl.id
                    ))
                }
                _ => {}
            },
            None if named => {
                return Err(format!("language `{}` declares no grammar", decl.id))
            }
            None => {}
        }
    }
    Ok(out)
}

/// The folder oxplow's author tools cache downloads in: `$OXPLOW_CACHE_DIR`,
/// else `$XDG_CACHE_HOME/oxplow`, else `~/.cache/oxplow`.
pub fn cache_dir() -> Result<PathBuf, String> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("OXPLOW_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(dir).join("oxplow"));
    }
    var("HOME")
        .or_else(|| var("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".cache/oxplow"))
        .ok_or_else(|| "no cache folder: set OXPLOW_CACHE_DIR".to_string())
}

/// FNV-1a of `text`, 16 hex digits: a repository's folder name.
fn short_hash(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// Where a grammar's clone lives: `<cache>/grammars/<repository hash>/<rev>`.
pub fn repo_cache(cache: &Path, source: &GrammarSource) -> PathBuf {
    let rev: String = source
        .rev
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache
        .join("grammars")
        .join(short_hash(&source.repository))
        .join(rev)
}

/// Where wasi-sdk `version` is kept: `<cache>/wasi-sdk/<version>`.
pub fn wasi_sdk_dir(cache: &Path, version: &str) -> PathBuf {
    cache.join("wasi-sdk").join(version)
}

/// wasi-sdk's name for a host (`std::env::consts` OS and ARCH), none when it
/// ships no build for it.
pub fn wasi_platform(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "x86_64") => "x86_64-macos",
        ("macos", "aarch64") => "arm64-macos",
        ("linux" | "freebsd", "x86_64") => "x86_64-linux",
        ("linux" | "freebsd", "aarch64") => "arm64-linux",
        ("windows", "x86_64") => "x86_64-windows",
        ("windows", "aarch64") => "arm64-windows",
        _ => return None,
    })
}

/// The release archive of wasi-sdk `version` for `platform`.
pub fn wasi_url(version: &str, platform: &str) -> String {
    format!(
        "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-{version}/wasi-sdk-{version}.0-{platform}.tar.gz"
    )
}

/// The `clang` arguments that compile a grammar's `src_dir` to `wasm`,
/// exporting `tree_sitter_<name>`: Zed's, `compile_grammar`.
pub fn compile_args(
    src_dir: &Path,
    wasm: &Path,
    name: &str,
    scanner: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-fPIC",
        "-shared",
        "-Os",
        &format!("-Wl,--export=tree_sitter_{name}"),
        "-Wl,--max-memory=4294967296",
        "-o",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    args.push(wasm.display().to_string());
    args.push("-I".into());
    args.push(src_dir.display().to_string());
    args.push(src_dir.join("parser.c").display().to_string());
    if let Some(scanner) = scanner {
        args.push(scanner.display().to_string());
    }
    args
}

/// The parser ABI generated when the source sets none.
pub const DEFAULT_ABI: u32 = 14;

/// Whether `base` (a grammar's folder) needs its parser generated: it has no
/// `src/parser.c` but has a `grammar.js`. A folder with neither can't be built.
pub fn needs_generate(base: &Path) -> Result<bool, String> {
    if base.join("src/parser.c").is_file() {
        return Ok(false);
    }
    if base.join("grammar.js").is_file() {
        return Ok(true);
    }
    // The same error as a missing parser with nothing to make it from.
    scanner_of(&base.join("src")).map(|_| false)
}

/// The tree-sitter CLI: `$TREE_SITTER_CLI` when set, else `tree-sitter` on
/// `PATH`. It is never downloaded or run through `npx`.
pub fn tree_sitter_cli(base: &Path) -> Result<PathBuf, String> {
    let missing = || {
        format!(
            "`{}` has no src/parser.c and no tree-sitter CLI to generate it: install it (`brew install tree-sitter-cli`, `cargo install tree-sitter-cli` or `npm i -g tree-sitter-cli`) or set TREE_SITTER_CLI",
            base.display()
        )
    };
    if let Some(cli) = std::env::var_os("TREE_SITTER_CLI").filter(|v| !v.is_empty()) {
        let cli = PathBuf::from(cli);
        return if cli.is_file() {
            Ok(cli)
        } else {
            Err(missing())
        };
    }
    let names: &[&str] = if cfg!(windows) {
        &["tree-sitter.exe", "tree-sitter.cmd", "tree-sitter"]
    } else {
        &["tree-sitter"]
    };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|candidate| candidate.is_file())
        .ok_or_else(missing)
}

/// The arguments of the generate step, run in the grammar's folder.
pub fn generate_args(abi: Option<u32>) -> Vec<String> {
    vec![
        "generate".into(),
        "--abi".into(),
        abi.unwrap_or(DEFAULT_ABI).to_string(),
    ]
}

/// Make `base`'s `src/parser.c` with the tree-sitter CLI when it has none
/// and has a `grammar.js`; nothing otherwise.
pub fn generate_parser(
    base: &Path,
    abi: Option<u32>,
    say: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !needs_generate(base)? {
        return Ok(());
    }
    let cli = tree_sitter_cli(base)?;
    let version = run(Command::new(&cli).arg("--version"), "tree-sitter --version")
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown version".into());
    let args = generate_args(abi);
    say(&format!(
        "generating src/parser.c: {} {} ({version}) in {}",
        cli.display(),
        args.join(" "),
        base.display()
    ));
    run(
        Command::new(&cli).args(&args).current_dir(base),
        "tree-sitter generate",
    )?;
    if base.join("src/parser.c").is_file() {
        Ok(())
    } else {
        Err("tree-sitter generate ran but wrote no src/parser.c".into())
    }
}

/// The scanner to compile with `src_dir`'s parser: its `scanner.c`, none
/// when it has none. A C++ scanner (`scanner.cc`) isn't supported.
pub fn scanner_of(src_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !src_dir.join("parser.c").is_file() {
        return Err(format!(
            "{} has no parser.c: run `tree-sitter generate` in the grammar, or point `source.path` at the folder whose `src/` holds it",
            src_dir.display()
        ));
    }
    if src_dir.join("scanner.cc").is_file() && !src_dir.join("scanner.c").is_file() {
        return Err(format!(
            "{} has a C++ scanner (scanner.cc); C++ scanners aren't supported: use a grammar with a C scanner",
            src_dir.display()
        ));
    }
    let scanner = src_dir.join("scanner.c");
    Ok(scanner.is_file().then_some(scanner))
}

/// The grammar's name, the symbol `tree_sitter_<name>` exports: its
/// `tree-sitter.json`'s first grammar, else the `name:` of its
/// `grammar.js`, else `language` (an oxplow language id, dashes as `_`).
pub fn grammar_name(base: &Path, language: &str) -> String {
    let from_json = std::fs::read_to_string(base.join("tree-sitter.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["grammars"][0]["name"].as_str().map(str::to_string));
    let from_js = || {
        let js = std::fs::read_to_string(base.join("grammar.js")).ok()?;
        let at = js.find("name:")?;
        let rest = js[at + "name:".len()..].trim_start();
        let quote = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"' | '`'))?;
        let end = rest[1..].find(quote)?;
        Some(rest[1..1 + end].to_string())
    };
    from_json
        .or_else(from_js)
        .unwrap_or_else(|| language.replace('-', "_"))
}

fn run(cmd: &mut Command, what: &str) -> Result<std::process::Output, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("{what}: couldn't run it ({e})"))?;
    if out.status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The grammar's repository at its `rev`, cloned shallow into the cache
/// (kept: a second build reads it again). Returns the clone's folder.
pub fn fetch(cache: &Path, source: &GrammarSource) -> Result<PathBuf, String> {
    let dir = repo_cache(cache, source);
    if dir.join(".git").is_dir() {
        // Already fetched: the same repository and rev.
        if run(
            Command::new("git")
                .args(["checkout", "--quiet", &source.rev])
                .current_dir(&dir),
            "git checkout",
        )
        .is_ok()
        {
            return Ok(dir);
        }
    } else {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        run(
            Command::new("git")
                .arg("init")
                .arg("--quiet")
                .current_dir(&dir),
            "git init",
        )?;
        run(
            Command::new("git")
                .args(["remote", "add", "origin", &source.repository])
                .current_dir(&dir),
            "git remote add",
        )?;
    }
    run(
        Command::new("git")
            .args(["fetch", "--quiet", "--depth", "1", "origin", &source.rev])
            .current_dir(&dir),
        &format!("git fetch {} {}", source.repository, source.rev),
    )?;
    run(
        Command::new("git")
            .args(["checkout", "--quiet", "FETCH_HEAD"])
            .current_dir(&dir),
        "git checkout",
    )?;
    Ok(dir)
}

/// wasi-sdk's `clang`: `WASI_SDK_PATH`'s when set, else the pinned release,
/// downloaded into the cache the first time.
pub fn wasi_clang(cache: &Path, say: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    let exe = if cfg!(windows) { "clang.exe" } else { "clang" };
    if let Some(path) = std::env::var_os("WASI_SDK_PATH").filter(|v| !v.is_empty()) {
        let clang = PathBuf::from(path).join("bin").join(exe);
        return if clang.is_file() {
            Ok(clang)
        } else {
            Err(format!("WASI_SDK_PATH has no {}", clang.display()))
        };
    }
    let dir = wasi_sdk_dir(cache, WASI_SDK_VERSION);
    let clang = dir.join("bin").join(exe);
    if clang.is_file() {
        return Ok(clang);
    }
    let platform =
        wasi_platform(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
            format!(
                "wasi-sdk has no build for {} {}: set WASI_SDK_PATH to one",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
    let url = wasi_url(WASI_SDK_VERSION, platform);
    say(&format!("downloading wasi-sdk {WASI_SDK_VERSION}: {url}"));
    let parent = dir.parent().unwrap_or(&dir).to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let archive = parent.join(format!("wasi-sdk-{WASI_SDK_VERSION}.tar.gz"));
    run(
        Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error", "-o"])
            .arg(&archive)
            .arg(&url),
        "curl",
    )?;
    let temp = parent.join(format!("{WASI_SDK_VERSION}.partial"));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
    run(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&temp),
        "tar",
    )?;
    let unpacked = std::fs::read_dir(&temp)
        .map_err(|e| e.to_string())?
        .flatten()
        .next()
        .ok_or("the wasi-sdk archive was empty")?
        .path();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&unpacked, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let _ = std::fs::remove_dir_all(&temp);
    let _ = std::fs::remove_file(&archive);
    if clang.is_file() {
        Ok(clang)
    } else {
        Err(format!("wasi-sdk unpacked without {}", clang.display()))
    }
}

/// Build one language's grammar; the size of the file written.
pub fn build(job: &BuildJob, cache: &Path, say: &mut dyn FnMut(&str)) -> Result<u64, String> {
    say(&format!(
        "{}: fetching {} at {}",
        job.language, job.source.repository, job.source.rev
    ));
    let repo = fetch(cache, &job.source)?;
    let base = match &job.source.path {
        Some(path) => repo.join(path),
        None => repo,
    };
    generate_parser(&base, job.source.abi, say)?;
    let src = base.join("src");
    let scanner = scanner_of(&src)?;
    let name = grammar_name(&base, &job.language);
    let clang = wasi_clang(cache, say)?;
    if let Some(parent) = job.wasm.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    say(&format!(
        "{}: compiling tree_sitter_{name}{}",
        job.language,
        if scanner.is_some() {
            " with its scanner"
        } else {
            ""
        }
    ));
    run(
        Command::new(&clang).args(compile_args(&src, &job.wasm, &name, scanner.as_deref())),
        "clang",
    )?;
    let size = std::fs::metadata(&job.wasm)
        .map_err(|e| format!("{}: {e}", job.wasm.display()))?
        .len();
    say(&format!(
        "{}: wrote {} ({size} bytes)",
        job.language,
        job.wasm.display()
    ));
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> GrammarSource {
        GrammarSource {
            repository: "https://github.com/acme/tree-sitter-acme".into(),
            rev: "v1.2/x".into(),
            path: None,
            abi: None,
        }
    }

    fn manifest(languages: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("extension.yaml"),
            format!("manifest: 2\nname: acme\nlanguages:\n{languages}"),
        )
        .unwrap();
        dir
    }

    /// `grammar: { wasm, source }` parses, `source` takes `path`, and an
    /// unknown key under it is refused.
    #[test]
    fn a_grammars_source_parses() {
        let dir = manifest(
            "  - { id: acme, title: Acme, grammar: { wasm: grammars/acme.wasm, source: { repository: 'https://x/y', rev: abc, path: sub } } }\n  - { id: other, title: Other, grammar: { wasm: grammars/o.wasm } }\n  - { id: plain, title: Plain }\n",
        );
        let all = jobs(dir.path(), None).unwrap();
        assert_eq!(
            all,
            vec![BuildJob {
                language: "acme".into(),
                wasm: dir.path().join("grammars/acme.wasm"),
                source: GrammarSource {
                    repository: "https://x/y".into(),
                    rev: "abc".into(),
                    path: Some("sub".into()),
                    abi: None
                },
            }]
        );
        let bad = manifest(
            "  - { id: a, title: A, grammar: { wasm: g.wasm, source: { repository: r, rev: v, branch: main } } }\n",
        );
        assert!(jobs(bad.path(), None).unwrap_err().contains("branch"));
    }

    #[test]
    fn a_named_language_must_have_something_to_build() {
        let dir = manifest(
            "  - { id: nosrc, title: N, grammar: { wasm: g.wasm } }\n  - { id: nowasm, title: N, grammar: { source: { repository: r, rev: v } } }\n  - { id: plain, title: P }\n",
        );
        assert!(jobs(dir.path(), Some("nosrc"))
            .unwrap_err()
            .contains("no `grammar.source`"));
        assert!(jobs(dir.path(), Some("nowasm"))
            .unwrap_err()
            .contains("no `grammar.wasm`"));
        assert!(jobs(dir.path(), Some("plain"))
            .unwrap_err()
            .contains("no grammar"));
        assert!(jobs(dir.path(), Some("ghost"))
            .unwrap_err()
            .contains("no language `ghost`"));
        assert!(jobs(dir.path(), None).unwrap().is_empty());
    }

    #[test]
    fn cache_paths_are_stable_and_safe() {
        let cache = Path::new("/c");
        let a = repo_cache(cache, &source());
        assert_eq!(a, repo_cache(cache, &source()));
        assert!(a.starts_with("/c/grammars"));
        assert_eq!(
            a.file_name().unwrap(),
            "v1.2_x",
            "a rev is a safe folder name"
        );
        let mut other = source();
        other.repository.push('2');
        assert_ne!(a.parent(), repo_cache(cache, &other).parent());
        assert_eq!(wasi_sdk_dir(cache, "34"), Path::new("/c/wasi-sdk/34"));
    }

    #[test]
    fn wasi_sdk_downloads_by_host() {
        assert_eq!(wasi_platform("macos", "aarch64"), Some("arm64-macos"));
        assert_eq!(wasi_platform("linux", "x86_64"), Some("x86_64-linux"));
        assert_eq!(wasi_platform("windows", "aarch64"), Some("arm64-windows"));
        assert_eq!(wasi_platform("plan9", "x86_64"), None);
        assert_eq!(
            wasi_url("34", "arm64-macos"),
            "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-arm64-macos.tar.gz"
        );
    }

    #[test]
    fn the_compile_flags_are_zeds() {
        let args = compile_args(
            Path::new("/g/src"),
            Path::new("/e/grammars/acme.wasm"),
            "acme",
            Some(Path::new("/g/src/scanner.c")),
        );
        assert_eq!(
            args,
            [
                "-fPIC",
                "-shared",
                "-Os",
                "-Wl,--export=tree_sitter_acme",
                "-Wl,--max-memory=4294967296",
                "-o",
                "/e/grammars/acme.wasm",
                "-I",
                "/g/src",
                "/g/src/parser.c",
                "/g/src/scanner.c",
            ]
        );
        assert_eq!(
            compile_args(Path::new("/g/src"), Path::new("o"), "a", None).len(),
            10
        );
    }

    #[test]
    fn a_scanner_is_c_and_a_cpp_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path();
        assert!(scanner_of(src).unwrap_err().contains("no parser.c"));
        std::fs::write(src.join("parser.c"), "").unwrap();
        assert_eq!(scanner_of(src).unwrap(), None);
        std::fs::write(src.join("scanner.c"), "").unwrap();
        assert_eq!(scanner_of(src).unwrap(), Some(src.join("scanner.c")));
        std::fs::remove_file(src.join("scanner.c")).unwrap();
        std::fs::write(src.join("scanner.cc"), "").unwrap();
        let err = scanner_of(src).unwrap_err();
        assert!(err.contains("C++ scanners aren't supported"), "{err}");
    }

    #[test]
    fn the_export_name_comes_from_the_grammar() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert_eq!(grammar_name(base, "my-lang"), "my_lang");
        std::fs::write(
            base.join("grammar.js"),
            "module.exports = grammar({\n  name: 'from_js',\n  rules: {}\n});\n",
        )
        .unwrap();
        assert_eq!(grammar_name(base, "x"), "from_js");
        std::fs::write(
            base.join("tree-sitter.json"),
            r#"{"grammars":[{"name":"from_json"}]}"#,
        )
        .unwrap();
        assert_eq!(grammar_name(base, "x"), "from_json");
    }

    /// With `OXPLOW_GRAMMAR_BUILD_E2E=1` and a wasi-sdk (`WASI_SDK_PATH`),
    /// a tiny grammar made here compiles to a wasm module that exports its
    /// parser; skipped otherwise (it needs the toolchain).
    #[test]
    fn a_tiny_grammar_builds_when_the_toolchain_is_there() {
        if std::env::var_os("OXPLOW_GRAMMAR_BUILD_E2E").is_none() {
            return;
        }
        let Some(sdk) = std::env::var_os("WASI_SDK_PATH") else {
            eprintln!("skipped: no WASI_SDK_PATH");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("parser.c"),
            "typedef struct TSLanguage TSLanguage;\nconst TSLanguage *tree_sitter_tiny(void) { return 0; }\n",
        )
        .unwrap();
        let wasm = dir.path().join("out/tiny.wasm");
        std::fs::create_dir_all(wasm.parent().unwrap()).unwrap();
        let clang = PathBuf::from(sdk).join("bin/clang");
        run(
            Command::new(clang).args(compile_args(&src, &wasm, "tiny", None)),
            "clang",
        )
        .unwrap();
        let bytes = std::fs::read(&wasm).unwrap();
        assert!(bytes.starts_with(b"\0asm"), "a wasm module");
        assert!(
            bytes
                .windows(b"tree_sitter_tiny".len())
                .any(|w| w == b"tree_sitter_tiny"),
            "it exports the parser"
        );
    }

    #[test]
    fn the_generate_step_is_decided_by_what_the_folder_has() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // Neither: the existing error.
        assert!(needs_generate(base).unwrap_err().contains("no parser.c"));
        std::fs::write(
            base.join("grammar.js"),
            "module.exports = grammar({name: 'x', rules: {}});",
        )
        .unwrap();
        assert_eq!(needs_generate(base), Ok(true));
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("src/parser.c"), "").unwrap();
        assert_eq!(
            needs_generate(base),
            Ok(false),
            "a committed parser.c is used as it is"
        );
    }

    #[test]
    fn generate_takes_the_abi_default_or_the_sources() {
        assert_eq!(generate_args(None), ["generate", "--abi", "14"]);
        assert_eq!(generate_args(Some(15)), ["generate", "--abi", "15"]);
        let dir = manifest(
            "  - { id: a, title: A, grammar: { wasm: g.wasm, source: { repository: r, rev: v, abi: 13 } } }\n",
        );
        assert_eq!(jobs(dir.path(), None).unwrap()[0].source.abi, Some(13));
    }

    /// The CLI is `$TREE_SITTER_CLI`, else `tree-sitter` on PATH; with
    /// neither the error says how to get it. One test, since it sets the
    /// process's environment. With a stand-in CLI that writes the parser, the
    /// generate step runs in the grammar's folder with the ABI, and says so.
    #[cfg(unix)]
    #[test]
    fn the_tree_sitter_cli_is_found_or_asked_for_and_never_fetched() {
        use std::os::unix::fs::PermissionsExt as _;
        let tools = tempfile::tempdir().unwrap();
        let base_dir = tempfile::tempdir().unwrap();
        let base = base_dir.path();
        std::fs::write(base.join("grammar.js"), "x").unwrap();
        let (old_cli, old_path) = (
            std::env::var_os("TREE_SITTER_CLI"),
            std::env::var_os("PATH"),
        );
        // Neither set.
        std::env::remove_var("TREE_SITTER_CLI");
        std::env::set_var("PATH", tools.path());
        let err = generate_parser(base, None, &mut |_| {}).unwrap_err();
        assert!(
            err.contains("has no src/parser.c and no tree-sitter CLI to generate it")
                && err.contains("brew install tree-sitter-cli")
                && err.contains("TREE_SITTER_CLI"),
            "{err}"
        );
        // A stand-in on PATH.
        let cli = tools.path().join("tree-sitter");
        std::fs::write(
            &cli,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'tree-sitter 0.99.0'; exit 0; fi\necho \"$@\" > args.txt\n/bin/mkdir -p src && : > src/parser.c\n",
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut said = Vec::new();
        generate_parser(base, Some(15), &mut |l| said.push(l.to_string())).unwrap();
        assert!(base.join("src/parser.c").is_file());
        assert_eq!(
            std::fs::read_to_string(base.join("args.txt"))
                .unwrap()
                .trim(),
            "generate --abi 15",
            "run in the grammar's folder"
        );
        assert!(
            said[0].contains("generate --abi 15") && said[0].contains("tree-sitter 0.99.0"),
            "{said:?}"
        );
        // A parser that's there isn't generated again.
        let mut again = Vec::new();
        generate_parser(base, None, &mut |l| again.push(l.to_string())).unwrap();
        assert!(again.is_empty());
        // $TREE_SITTER_CLI names one that isn't there.
        std::env::set_var("TREE_SITTER_CLI", tools.path().join("missing"));
        std::fs::remove_file(base.join("src/parser.c")).unwrap();
        assert!(generate_parser(base, None, &mut |_| {})
            .unwrap_err()
            .contains("no tree-sitter CLI"));
        match old_cli {
            Some(v) => std::env::set_var("TREE_SITTER_CLI", v),
            None => std::env::remove_var("TREE_SITTER_CLI"),
        }
        match old_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
}
