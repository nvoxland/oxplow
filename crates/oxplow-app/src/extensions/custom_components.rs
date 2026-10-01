//! `custom_components:` (experimental — a private extension only; P6b.D1):
//! a web bundle a `viz: custom` lens renders in a sandboxed frame. The
//! frame has no origin, no network and no daemon token, so it reaches
//! only what it declares: the lenses it may query (`assets`, lens ids)
//! and the commands it may invoke (`commands`). That sandbox is the
//! consent — the bundle runs without a person's approval.
//!
//! ```yaml
//! custom_components:
//!   - id: burndown
//!     title: Burndown
//!     bundle: components/burndown        # default components/<id>; holds index.html
//!     assets: [open-tasks, oxplow-analytics/visits]   # a bare slug is this extension's
//!     commands: [work_item.transition]
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::manifest_v2::{at, entry_line, key_line};

/// The most a bundle may hold.
pub const MAX_BUNDLE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_BUNDLE_FILES: usize = 256;

/// A declared component (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CustomComponent {
    /// `[a-z0-9-]+`, unique in the extension; a lens names it in
    /// `custom.component`.
    pub id: String,
    pub extension: String,
    pub title: Option<String>,
    /// The bundle's folder inside the extension, holding `index.html`.
    pub bundle: String,
    /// Lens ids (`<extension>/<slug>`) the component may query.
    pub assets: Vec<String>,
    /// Commands the component may invoke.
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentFile {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    bundle: Option<String>,
    #[serde(default)]
    assets: Vec<String>,
    #[serde(default)]
    commands: Vec<String>,
}

/// What a bundle folder holds, as far as loading cares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BundleStat {
    pub has_index: bool,
    pub files: usize,
    pub bytes: u64,
    /// A symlink found inside it (its path in the bundle), if any.
    pub symlink: Option<String>,
}

/// Walk `dir` (a bundle on disk): `None` when it isn't a directory. A
/// symlink anywhere — the folder itself included — is reported, never
/// followed.
pub fn stat_bundle(dir: &Path) -> Option<BundleStat> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.file_type().is_symlink() {
        return Some(BundleStat {
            symlink: Some(".".into()),
            ..BundleStat::default()
        });
    }
    if !meta.is_dir() {
        return None;
    }
    let mut stat = BundleStat {
        has_index: std::fs::symlink_metadata(dir.join("index.html")).is_ok_and(|m| m.is_file()),
        ..BundleStat::default()
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                stat.symlink = Some(
                    path.strip_prefix(dir)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
                return Some(stat);
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                stat.files += 1;
                stat.bytes += meta.len();
            }
        }
    }
    Some(stat)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A relative path inside the extension, without `..`.
fn inside(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Parse `custom_components:`: the valid ones and an error (`file:line:
/// …`) for each broken one. `stat` describes a bundle folder by its path
/// inside the extension (`None`: no such folder).
pub fn parse_custom_components(
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    stat: &dyn Fn(&str) -> Option<BundleStat>,
) -> (Vec<CustomComponent>, Vec<String>) {
    let block = key_line(manifest, "custom_components");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`custom_components` must be a list")],
        );
    };
    let kinds = oxplow_domain::refs::kind::core_kinds();
    let mut out: Vec<CustomComponent> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let c: ComponentFile = match serde_yaml::from_value(item.clone()) {
            Ok(c) => c,
            Err(e) => {
                errors.push(at(file, block, format!("custom component: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "custom_components", "id", &c.id).or(block);
        let bundle = c
            .bundle
            .clone()
            .unwrap_or_else(|| format!("components/{}", c.id));
        let assets: Vec<String> = c
            .assets
            .iter()
            .map(|a| {
                if a.contains('/') {
                    a.clone()
                } else {
                    format!("{extension}/{a}")
                }
            })
            .collect();
        let problem = if !valid_id(&c.id) {
            Some(format!(
                "custom component id `{}` must be lowercase letters, digits and dashes",
                c.id
            ))
        } else if out.iter().any(|o| o.id == c.id) {
            Some(format!("custom component `{}` is declared twice", c.id))
        } else if !inside(&bundle) {
            Some(format!(
                "custom component `{}`: bundle `{bundle}` must be a folder inside the extension",
                c.id
            ))
        } else if let Some(bad) = assets.iter().find(|a| {
            // A lens id alone: params, a revision or a fragment would make
            // it a different ref than the one `query(asset)` names.
            a.contains(['?', '@', '#'])
                || oxplow_domain::refs::grammar::CanonicalRef::parse(&format!("lens:{a}"))
                    .map_or(true, |r| kinds.validate(&r).is_err())
        }) {
            Some(format!(
                "custom component `{}`: asset `{bad}` isn't a lens id (`<slug>` or `<extension>/<slug>`)",
                c.id
            ))
        } else if let Some(bad) = c
            .commands
            .iter()
            .find(|n| oxplow_domain::CommandSpec::validate_name(n).is_err())
        {
            Some(format!(
                "custom component `{}`: `{bad}` isn't a command name",
                c.id
            ))
        } else {
            match stat(&bundle) {
                None => Some(format!(
                    "custom component `{}`: bundle `{bundle}` isn't a folder in the extension",
                    c.id
                )),
                Some(s) if s.symlink.is_some() => Some(format!(
                    "custom component `{}`: bundle `{bundle}` holds a symlink (`{}`); a bundle is \
                     plain files",
                    c.id,
                    s.symlink.unwrap_or_default()
                )),
                Some(s) if !s.has_index => Some(format!(
                    "custom component `{}`: bundle `{bundle}` has no `index.html`",
                    c.id
                )),
                Some(s) if s.files > MAX_BUNDLE_FILES || s.bytes > MAX_BUNDLE_BYTES => {
                    Some(format!(
                        "custom component `{}`: bundle `{bundle}` is {} files, {} bytes; the most \
                         is {MAX_BUNDLE_FILES} files, {MAX_BUNDLE_BYTES} bytes",
                        c.id, s.files, s.bytes
                    ))
                }
                Some(_) => None,
            }
        };
        match problem {
            Some(p) => errors.push(at(file, line, p)),
            None => out.push(CustomComponent {
                id: c.id,
                extension: extension.to_string(),
                title: c.title,
                bundle,
                assets,
                commands: c.commands,
            }),
        }
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use crate::extensions::{load_extensions, Extension, LensViz};
    use std::path::Path;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const LENS: &str = "title: Burn\nquery: SELECT 1 AS day, 3 AS remaining\nviz: custom\ncustom: { component: burndown, props: { color: accent } }\n";

    /// Extension `x` (`sharing`) declaring `components` (the block's
    /// entries), with a `burndown` bundle and the `burn` lens.
    fn load(root: &Path, sharing: &str, components: &str) -> Extension {
        write(
            root,
            "oxplow/extensions/x/extension.yaml",
            &format!(
                "manifest: 2\nname: x\nsharing: {sharing}\nengine: \">=0.1\"\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\ncustom_components:\n{components}"
            ),
        );
        write(
            root,
            "oxplow/extensions/x/components/burndown/index.html",
            "<!doctype html><script src=\"app.js\"></script>",
        );
        write(root, "oxplow/extensions/x/components/burndown/app.js", "1");
        write(root, "oxplow/extensions/x/lenses/burn.yaml", LENS);
        write(
            root,
            "oxplow/extensions/x/lenses/open-tasks.yaml",
            "title: Open\nquery: SELECT 1\n",
        );
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == "x")
            .unwrap()
    }

    #[test]
    fn a_private_extension_loads_a_component_and_its_lens() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(
            d.path(),
            "private",
            "  - { id: burndown, title: Burndown, assets: [open-tasks, oxplow-analytics/visits], commands: [work_item.transition] }\n",
        );
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let c = &ext.custom_components[0];
        assert_eq!(c.bundle, "components/burndown");
        assert_eq!(c.assets, vec!["x/open-tasks", "oxplow-analytics/visits"]);
        let lens = ext.lenses.iter().find(|l| l.slug == "burn").unwrap();
        assert_eq!(lens.viz, LensViz::Custom);
        assert_eq!(
            lens.custom.as_ref().unwrap().props.as_ref().unwrap()["color"],
            "accent"
        );
    }

    #[test]
    fn a_broken_component_is_an_error_at_its_line() {
        for (entry, setup, says) in [
            ("{ id: Bad Id }", "", "must be lowercase"),
            (
                "{ id: c, bundle: ../x }",
                "",
                "must be a folder inside the extension",
            ),
            (
                "{ id: c, bundle: nowhere }",
                "",
                "isn't a folder in the extension",
            ),
            ("{ id: c, bundle: empty }", "empty", "has no `index.html`"),
            ("{ id: c, bundle: linked }", "symlink", "holds a symlink"),
            (
                "{ id: c, bundle: \".\" }",
                "",
                "must be a folder inside the extension",
            ),
            (
                "{ id: c, bundle: aliased }",
                "linked-folder",
                "holds a symlink",
            ),
            (
                "{ id: c, bundle: dirindex }",
                "index-dir",
                "has no `index.html`",
            ),
            ("{ id: c, bundle: big }", "big", "the most is 256 files"),
            (
                "{ id: c, bundle: components/burndown, assets: [nope] }",
                "",
                "asset `x/nope` isn't in this extension's lenses/",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"Not A Lens\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"open-tasks?stream_id=2\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, assets: [\"open-tasks#x\"] }",
                "",
                "isn't a lens id",
            ),
            (
                "{ id: c, bundle: components/burndown, commands: [Nope] }",
                "",
                "isn't a command name",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            let base = d.path().join("oxplow/extensions/x");
            match setup {
                "empty" => write(d.path(), "oxplow/extensions/x/empty/readme.txt", "x"),
                "symlink" => {
                    write(d.path(), "oxplow/extensions/x/linked/index.html", "x");
                    std::os::unix::fs::symlink("/etc/hosts", base.join("linked/hosts")).unwrap();
                }
                "linked-folder" => {
                    write(d.path(), "oxplow/extensions/x/real/index.html", "x");
                    std::os::unix::fs::symlink(base.join("real"), base.join("aliased")).unwrap();
                }
                "index-dir" => write(
                    d.path(),
                    "oxplow/extensions/x/dirindex/index.html/a.js",
                    "x",
                ),
                "big" => {
                    write(d.path(), "oxplow/extensions/x/big/index.html", "x");
                    for i in 0..256 {
                        write(d.path(), &format!("oxplow/extensions/x/big/f{i}.js"), "x");
                    }
                }
                _ => {}
            }
            let ext = load(
                d.path(),
                "private",
                &format!("  - {entry}\n  - {{ id: burndown }}\n"),
            );
            let errs = ext.errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("extension.yaml:"),
                "{entry}: {errs}"
            );
            assert_eq!(
                ext.custom_components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["burndown"],
                "{entry}"
            );
        }
    }

    #[test]
    fn a_shared_extension_may_not_declare_components_and_its_custom_lens_drops() {
        let d = tempfile::tempdir().unwrap();
        let ext = load(d.path(), "shared", "  - { id: burndown }\n");
        let errs = ext.errors.join("\n");
        assert!(
            errs.contains("`custom_components` is experimental"),
            "{errs}"
        );
        assert!(ext.custom_components.is_empty());
        assert!(
            errs.contains("`burndown` isn't one of this extension's `custom_components`"),
            "{errs}"
        );
        assert!(ext.lenses.iter().all(|l| l.slug != "burn"));
    }

    #[test]
    fn a_custom_lens_needs_its_component_and_a_query() {
        for (lens, says) in [
            (
                "title: B\nquery: SELECT 1\nviz: custom\n",
                "needs `custom: { component: <id> }`",
            ),
            (
                "title: B\nviz: custom\ncustom: { component: burndown }\n",
                "needs a `query`",
            ),
        ] {
            let d = tempfile::tempdir().unwrap();
            write(d.path(), "oxplow/extensions/x/lenses/other.yaml", lens);
            let ext = load(d.path(), "private", "  - { id: burndown }\n");
            assert!(
                ext.errors.join("\n").contains(says),
                "{lens}: {:?}",
                ext.errors
            );
            assert!(ext.lenses.iter().all(|l| l.slug != "other"));
        }
        let spec: crate::extensions::LensSpec = serde_json::from_value(serde_json::json!({
            "title": "B", "query": "SELECT 1", "viz": "custom"
        }))
        .unwrap();
        assert!(
            crate::extensions::spec_problem(&spec).is_some(),
            "an answer can't carry one"
        );
    }

    #[tokio::test]
    async fn validation_checks_its_commands_reads_as_a_table_and_nudges_a_lookalike() {
        let d = tempfile::tempdir().unwrap();
        load(
            d.path(),
            "private",
            "  - { id: burndown, commands: [nope.cmd, work_item.transition] }\n",
        );
        write(
            d.path(),
            "oxplow/extensions/x/lenses/burn.yaml",
            "title: Burn\nquery: SELECT 1 AS day, 3 AS remaining\nviz: custom\ncustom: { component: burndown }\nchart: { x: day, y: remaining }\n",
        );
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let cat = crate::extension_catalog::ExtensionCatalog::new();
        let schema = |n: &str| {
            (n == "work_item.transition").then(|| serde_json::json!({ "type": "object" }))
        };
        let v = crate::extensions::validate_extension(&layer, &cat, d.path(), "x", Some(&schema))
            .await
            .unwrap();
        let errs = v.errors.join("\n");
        assert!(
            errs.contains("declares command `nope.cmd`, which isn't registered"),
            "{errs}"
        );
        assert!(!errs.contains("work_item.transition"), "{errs}");
        assert!(
            v.warnings
                .join("\n")
                .contains("the kit's `chart` viz may already cover it"),
            "{:?}",
            v.warnings
        );
        let run = crate::extensions::run_lens(
            &layer,
            &cat,
            d.path(),
            "x/burn",
            Default::default(),
            &Default::default(),
        )
        .await
        .unwrap();
        let text = crate::lens_text::render(&run, &Default::default());
        assert!(
            text.starts_with("(custom component `x/burndown`; its table rendering)\n"),
            "{text}"
        );
    }
}
