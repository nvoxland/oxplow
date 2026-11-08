//! One contribution to an extension: what `extension add <kind> <id>` appends
//! to a manifest and writes beside it, and the pieces `extension new <kind>`
//! builds an extension from (a manifest header and one contribution).

use std::path::Path;

use oxplow_app::extension::extensions::EXTENSIONS_DIR;
use oxplow_domain::contribution::{extension_namespace, ContributionKind};
use oxplow_domain::uri::build::work_item_uri;

use crate::{title_case, Kind, SdkError};

/// What one contribution of a kind adds to an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    /// Per manifest top-level list (`collectors`, `datasets`, `commands`, …)
    /// the entry to append to it: one list item, indented two spaces,
    /// comments included. A kind whose files are all it needs (a page) has
    /// none.
    pub entries: Vec<(&'static str, String)>,
    /// Permissions its scripts need, as the manifest's `permissions:` reads.
    pub permissions: Vec<&'static str>,
    /// Files to write, as (path inside the extension folder, contents).
    pub files: Vec<(String, String)>,
}

/// The kinds `extension add` takes.
pub const ADDABLE: &str =
    "`page`, `collector`, `command`, `effect`, `component`, `advisory` or `skill`";

/// Every service the table offers, with the builtin that reaches it.
fn services_list() -> String {
    oxplow_domain::service::SERVICES
        .iter()
        .map(|service| {
            let via = match service.builtins.first() {
                Some(builtin) if service.builtins.len() > 1 => format!(" ({builtin}() and more)"),
                Some(builtin) => format!(" ({builtin}())"),
                None => String::new(),
            };
            format!("{}{via}", service.scope)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The contribution of `kind` named `id` to an extension in `namespace`. A kind
/// that has none of its own (an extension, a provider, a metric, a policy, a
/// panel, a job) is refused.
pub fn contribution(kind: Kind, namespace: &str, id: &str) -> Result<Contribution, SdkError> {
    build(kind, namespace, id, None)
}

/// [`contribution`], with the script path and menu label a command gets when
/// `new command` makes it (the one starter `new` has always written).
pub(crate) fn build(
    kind: Kind,
    namespace: &str,
    id: &str,
    command: Option<(&str, &str)>,
) -> Result<Contribution, SdkError> {
    let ns = extension_namespace(namespace);
    let work_item = work_item_uri("oxplow", "tsk1");
    let mut files: Vec<(String, String)> = Vec::new();
    let mut write = |rel: &str, body: String| -> Result<(), SdkError> {
        files.push((rel.to_string(), body));
        Ok(())
    };
    let (entries, permissions): (Vec<(&'static str, String)>, Vec<&'static str>) = match kind {
        Kind::Page => {
        write(
            &format!("pages/{id}.yaml"),
            format!(
                "title: {title}\n\
                 description: \"TODO: what this page answers.\"\n\
                 category: Work\n\
                 params:\n\
                 \x20 # Filled in with the viewer's stream unless a value is given.\n\
                 \x20 - {{ name: stream_id, label: Stream }}\n\
                 answers:\n\
                 \x20 # Questions this page answers, in a person's words: how an agent finds it.\n\
                 \x20 - {{ question: \"TODO: what is open in this stream?\", params: {{ stream_id: 1 }} }}\n\
                 loaders:\n\
                 \x20 items:\n\
                 \x20   sql: |\n\
                 \x20     SELECT w.uri, w.title, w.state, w.updated_at\n\
                 \x20     FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id\n\
                 \x20     WHERE t.stream_id = :stream_id AND w.state IN ('todo', 'in_progress', 'blocked')\n\
                 \x20     ORDER BY w.updated_at DESC\n\
                 view:\n\
                 \x20 viz: table\n\
                 \x20 from: items\n\
                 \x20 columns:\n\
                 \x20   - {{ key: title, label: Item, link: {{ kind: page, from: uri }} }}\n\
                 \x20   - {{ key: state }}\n\
                 \x20 actions:\n\
                 \x20   # In each row's right-click menu: a command run as the person, the\n\
                 \x20   # row's values bound in (`{{{{row.<column>}}}}`).\n\
                 \x20   - {{ id: start, label: Start, command: oxplow.work_item.transition, input: {{ uri: \"{{{{row.uri}}}}\", to: in_progress }}, row: true }}\n\
                 \x20 empty: No open work items in this stream.\n",
                title = title_case(id)
            ),
        )?;
            (Vec::new(), Vec::new())
        }
        Kind::Collector => {
            let services = services_list();
            let u = id.replace('-', "_");
            let entity = u
                .strip_suffix('s')
                .filter(|e| !e.is_empty())
                .unwrap_or(&u)
                .to_string();
            let dataset = format!("open_{u}");
            write(
                &format!("collectors/{id}.star"),
                "# Gets {\"rows\": [...]} (its `input` query's) and returns the entities it\n\
                 # declares. Sandboxed: no files, network or clock.\n\
                 def transform(x):\n\
                 \x20   return {\"entities\": {\"@ENT@\": [\n\
                 \x20       {\"uri\": r[\"uri\"], \"title\": r[\"title\"]}\n\
                 \x20       for r in x[\"rows\"]\n\
                 \x20       if r[\"state\"] != \"done\"\n\
                 \x20   ]}}\n"
                    .replace("@ENT@", &entity),
            )?;
            write(
                &format!("datasets/{dataset}.sql"),
                "SELECT uri, title FROM ref('@ENT@')\n".replace("@ENT@", &entity),
            )?;
            (
                vec![
                    (
                        "collectors",
                        format!(
                            "\x20 # Runs its script over its `input` rows when synced (`on: manual`;\n\
             \x20 # `{{ every: 15m }}` or `{{ on: [snapshot.taken] }}` run it by themselves).\n\
             \x20 - id: {id}\n\
             \x20   runtime: starlark\n\
             \x20   entry: collectors/{id}.star\n\
             \x20   # Services its script may use: {services}.\n\
             \x20   needs: []\n\
             \x20   input: \"SELECT uri, title, state FROM v_work_item\"\n\
             \x20   entities:\n\
             \x20     - {{ name: {entity}, key: uri, columns: {{ uri: text, title: text }} }}\n"
                        ),
                    ),
                    (
                        "datasets",
                        format!(
                            "\x20 # v_{ns}_{dataset}: SQL over the entity (`ref('item')`), checked at load.\n\
             \x20 - name: {dataset}\n\
             \x20   version: 1\n\
             \x20   description: \"TODO: what these rows are.\"\n\
             \x20   fields:\n\
             \x20     - {{ name: uri, type: TEXT, doc: \"The work item (oxplow:oxplow/work_item?id=<id>&provider=<provider>).\" }}\n\
             \x20     - {{ name: title, type: TEXT, doc: \"Its title.\" }}\n"
                        ),
                    ),
                ],
                Vec::new(),
            )
        }
        Kind::Command => {
            let (file, label) = match command {
                Some((file, label)) => (file.to_string(), label.to_string()),
                None => {
                    let verb = id.rsplit('.').next().unwrap_or(id);
                    let area = id.split('.').next().unwrap_or(id);
                    (
                        format!("handlers/{}.star", id.replace('.', "_")),
                        format!("{} {}", title_case(verb), title_case(area)),
                    )
                }
            };
        write(
            &file,
            "# Gets {\"input\": {...}, \"rows\": [...]} and returns the core commands to run\n\
             # as the caller (in one transaction when every one can, else as steps) — or {\"refuse\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"oxplow.work_item.comment\", \"input\": {\"uri\": x[\"input\"][\"uri\"], \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?;
            (
                vec![(
                    "commands",
                    format!(
                        "\x20 # Registered as {ns}.{id} (<namespace>.<area>.<verb>); its script composes core commands, run as the\n\
             \x20 # caller: in one transaction when every one can, else as steps (a work list's verbs run outside it).\n\
             \x20 - name: {id}\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item.\"\n\
             \x20   input_schema:\n\
             \x20     type: object\n\
             \x20     required: [uri]\n\
             \x20     properties:\n\
             \x20       uri: {{ type: string, description: \"The work item (oxplow:oxplow/work_item?id=<id>&provider=<provider>).\" }}\n\
             \x20     additionalProperties: false\n\
             \x20   entry: {file}\n\
             \x20   # Who may run it: a person, an agent, a page (each child command checks its own too).\n\
             \x20   \
             \x20   # On a work item's page (Commands) and a row's right-click.\n\
             \x20   ui: {{ label: {label}, about: oxplow/work_item, input: {{ uri: \"{{{{uri}}}}\" }} }}\n\
             \x20   examples:\n\
             \x20     - {{ name: happy, input: {{ uri: \"{work_item}\" }}, expect_commands: [oxplow.work_item.comment] }}\n"
                    ),
                )],
                vec!["commands:oxplow.work_item.comment"],
            )
        }
        Kind::Effect => {
        write(
            &format!("effects/{id}.star"),
            "# Gets {\"event\": {id, type, v, seq, at, source, subject, anchors, payload, cause}}\n\
             # (the event envelope, at its type's newest version) and returns the\n\
             # commands to run — or {\"skip\": \"why\"}. No I/O.\n\
             def transform(x):\n\
             \x20   uri = x[\"event\"][\"payload\"][\"work_item\"]\n\
             \x20   return {\"commands\": [\n\
             \x20       {\"name\": \"oxplow.work_item.comment\", \"input\": {\"uri\": uri, \"body\": \"TODO: the note\"}},\n\
             \x20   ]}\n"
                .to_string(),
        )?;
            (
                vec![(
                    "effects",
                    "\x20 # Reacts to a logged event by composing commands, run with an agent's\n\
             \x20 # rights under this extension's permissions, and only on events logged\n\
             \x20 # after it started; a command that asks becomes a proposal.\n\
             \x20 - id: on-done\n\
             \x20   summary: \"TODO: what it does. Here: comment on a work item when it's done.\"\n\
             \x20   on: [work_item.state_changed]\n\
             \x20   where: { to: done }\n\
             \x20   entry: effects/on-done.star\n"
                        .replace("on-done", id),
                )],
                vec!["commands:oxplow.work_item.comment"],
            )
        }
        Kind::Component => {
            write(
                &format!("pages/{id}.yaml"),
                format!(
                    "title: {title}\n\
                     description: \"TODO: what this view answers.\"\n\
                     category: Work\n\
                     params:\n\
                     \x20 # Filled in with the viewer's stream unless a value is given.\n\
                     \x20 - {{ name: stream_id, label: Stream }}\n\
                     answers:\n\
                     \x20 - {{ question: \"TODO: how many open tasks are there?\", params: {{ stream_id: 1 }} }}\n\
                     loaders:\n\
                     \x20 # The rows the component is fed (`component.data.items`), and what an\n\
                     \x20 # agent reads: agents always see the table, never the frame.\n\
                     \x20 items:\n\
                     \x20   sql: |\n\
                     \x20     SELECT w.uri, w.title, w.state\n\
                     \x20     FROM v_work_item w JOIN v_thread t ON t.id = w.thread_id\n\
                     \x20     WHERE t.stream_id = :stream_id AND w.state IN ('todo', 'in_progress', 'blocked')\n\
                     \x20     ORDER BY w.updated_at DESC\n\
                     view:\n\
                     \x20 component: {{ id: {id} }}\n",
                    title = title_case(id)
                ),
            )?;
            write(
                &format!("components/{id}/index.html"),
                "<!doctype html>\n\
                 <meta charset=\"utf-8\">\n\
                 <!-- oxplow's look: the kit's classes over the theme, set by applyTheme(). -->\n\
                 <link rel=\"stylesheet\" href=\"/component-lib/oxplow-kit.css\">\n\
                 <div id=\"out\" class=\"ox-muted\"></div>\n\
                 <!-- oxplow's client library (it defines `oxplow`), then this bundle's own\n\
                 \x20    script. Both are plain scripts: a sandboxed frame can't load modules. -->\n\
                 <script src=\"/component-lib/oxplow-component.js\"></script>\n\
                 <script src=\"app.js\"></script>\n"
                    .to_string(),
            )?;
            write(
                &format!("components/{id}/app.js"),
                "// Runs in a sandboxed frame. `oxplow.connect()` resolves once oxplow has\n\
                 // handed over the page's loaders:\n\
                 //   component.data                    each loader's rows by name ({ items: { columns, rows } })\n\
                 //   component.onUpdate(fn)            a loader changed: fn({ data, params, changed })\n\
                 //   component.invoke(command, input)  a command listed in `commands`; oxplow shows\n\
                 //                                     and records a failure itself (draw only a `field` error)\n\
                 //   component.navigate(ref)           open one of oxplow's pages\n\
                 oxplow.connect().then((component) => {\n\
                 \x20 component.applyTheme();\n\
                 \x20 const out = document.getElementById(\"out\");\n\
                 \x20 const render = (data) => {\n\
                 \x20   out.textContent = data.items.rows.length + \" open tasks\";\n\
                 \x20 };\n\
                 \x20 render(component.data);\n\
                 \x20 component.onUpdate(({ data }) => render(data));\n\
                 });\n"
                    .to_string(),
            )?;
            (
                vec![(
                    "custom_components",
                    format!(
                        "\x20 # A web bundle (components/{id}/) a page's `component` node draws in a\n\
             \x20 # sandboxed frame: scripts only, no network, no storage. It is fed the\n\
             \x20 # page's loaders and may run the commands in `commands` — as the\n\
             \x20 # person looking at it — and nothing else.\n\
             \x20 - id: {id}\n\
             \x20   commands: []\n"
                    ),
                )],
                Vec::new(),
            )
        }
        Kind::Advisory => (
            vec![(
                "advisories",
                format!(
                    "\
             \x20 # A {advisory}: a query run at a moment (`on`: post-tool-use, prompt or\n\
             \x20 # turn-end) whose rows reach the agent (`audience: person` raises them in\n\
             \x20 # Alerts instead). Each row's `message` is what it hears; `once_per` says how\n\
             \x20 # often. Params: :thread_id, :stream_id, :turn_id, :effort_id. A shared\n\
             \x20 # extension's advisories run once a person approves them.\n\
             \x20 - id: {id}\n\
             \x20   on: turn-end\n\
             \x20   once_per: effort\n\
             \x20   query: |\n\
             \x20     SELECT printf('TODO: what the agent should hear. Here: this effort has changed %d files.',\n\
             \x20                    count(DISTINCT f.path)) AS message\n\
             \x20     FROM v_effort e JOIN v_effort_file f ON f.effort_id = e.id\n\
             \x20     WHERE e.id = :effort_id AND e.ended_at IS NULL\n\
             \x20     HAVING count(DISTINCT f.path) >= 15\n",
                    advisory = ContributionKind::Advisory.spec().title.to_lowercase()
                ),
            )],
            vec!["agent.write"],
        ),
        Kind::Skill => {
        write(
            &format!("skills/{id}/SKILL.md"),
            format!(
                "---\nname: {id}\ndescription: \"TODO: when the agent should load this.\"\n---\n\n\
                 # {title}\n\nTODO: what the agent should know or do.\n",
                title = title_case(id)
            ),
        )?;
            (
                vec![(
                    "skills",
                    format!(
                        "\
             \x20 # A {skill} the coding agent loads when it's relevant; `kind: command`\n\
             \x20 # makes it a slash command a person runs. `needs: [work_items]` offers it only\n\
             \x20 # while that capability is active.\n\
             \x20 - name: {id}\n\
             \x20   file: skills/{id}/SKILL.md\n",
                        skill = ContributionKind::Skill.spec().title.to_lowercase()
                    ),
                )],
                vec!["agent.write"],
            )
        }
        _ => {
            return Err(SdkError::Invalid(format!(
                "`add` takes {ADDABLE}; make a whole new extension with `new`"
            )))
        }
    };
    Ok(Contribution {
        entries,
        permissions,
        files,
    })
}

/// Add one `kind` contribution named `id` to an extension already in the
/// project (`extension` when it is the only one under
/// `.oxplow/extensions/`, else named): its files are written (none may exist
/// already) and its entries appended to the manifest's lists, a text edit
/// that keeps the manifest's comments. An `id` the list already holds is
/// refused, and nothing is written when anything is refused. Returns the
/// extension and what was written.
pub fn add_contribution(
    root: &Path,
    kind: Kind,
    id: &str,
    extension: Option<&str>,
) -> Result<crate::Scaffolded, SdkError> {
    let valid_id = id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c));
    if !valid_id {
        return Err(SdkError::Invalid(format!(
            "`{id}` must start with a letter and be lowercase letters, digits, `-`, `_` and `.` (e.g. `sales`)"
        )));
    }
    let name = resolve_extension(root, extension)?;
    let rel_dir = format!("{EXTENSIONS_DIR}/{name}");
    let manifest_path = root.join(&rel_dir).join("extension.yaml");
    let original = std::fs::read_to_string(&manifest_path)?;
    let parsed: serde_yaml::Value = serde_yaml::from_str(&original)
        .map_err(|e| SdkError::Invalid(format!("{rel_dir}/extension.yaml doesn't parse: {e}")))?;
    // What it names (`v_<ns>_…`, `<ns>.<command>`) is the manifest's
    // `namespace:`, the name's when it has none.
    let namespace = parsed
        .get("namespace")
        .and_then(serde_yaml::Value::as_str)
        .map_or_else(|| extension_namespace(&name), str::to_string);
    let made = contribution(kind, &namespace, id)?;

    let mut text = original;
    for (block, entry) in &made.entries {
        // The id the entry names (`id:` or `name:`) must be new in its list.
        let item: serde_yaml::Value = serde_yaml::from_str(&dedent(entry)).unwrap_or_default();
        let named = item
            .get(0)
            .and_then(|i| i.get("id").or_else(|| i.get("name")))
            .and_then(serde_yaml::Value::as_str)
            .map(str::to_string);
        if let Some(named) = named {
            let held = parsed
                .get(*block)
                .and_then(serde_yaml::Value::as_sequence)
                .is_some_and(|items| {
                    items.iter().any(|i| {
                        i.get("id")
                            .or_else(|| i.get("name"))
                            .and_then(serde_yaml::Value::as_str)
                            == Some(named.as_str())
                    })
                });
            if held {
                return Err(SdkError::Invalid(format!(
                    "{rel_dir}/extension.yaml: `{block}` already has `{named}`; pick another id"
                )));
            }
        }
        text = oxplow_config::append_list_entry(&text, block, entry)
            .map_err(|e| SdkError::Invalid(format!("{rel_dir}/extension.yaml: {e}")))?;
    }
    for (rel, _) in &made.files {
        if root.join(&rel_dir).join(rel).exists() {
            return Err(SdkError::Invalid(format!(
                "{rel_dir}/{rel} already exists; pick another id or edit it in place"
            )));
        }
    }
    text = add_permissions(&text, &made.permissions)
        .map_err(|e| SdkError::Invalid(format!("{rel_dir}/extension.yaml: {e}")))?;

    let mut written = Vec::new();
    for (rel, body) in &made.files {
        let path = root.join(&rel_dir).join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
        written.push(format!("{rel_dir}/{rel}"));
    }
    std::fs::write(&manifest_path, text)?;
    written.push(format!("{rel_dir}/extension.yaml"));
    Ok(crate::Scaffolded {
        name,
        dir: rel_dir,
        files: written,
    })
}

/// `extension` if given (it must exist), else the only extension in the
/// project.
fn resolve_extension(root: &Path, extension: Option<&str>) -> Result<String, SdkError> {
    let dir = root.join(EXTENSIONS_DIR);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("extension.yaml").is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    match (extension, names.as_slice()) {
        (Some(name), _) if names.iter().any(|n| n == name) => Ok(name.to_string()),
        (Some(name), _) => Err(SdkError::NotFound(name.to_string())),
        (None, [only]) => Ok(only.clone()),
        (None, []) => Err(SdkError::Invalid(format!(
            "no extension under {EXTENSIONS_DIR}/: make one with `extension new`"
        ))),
        (None, several) => Err(SdkError::Invalid(format!(
            "which extension? one of {}",
            several.join(", ")
        ))),
    }
}

/// An entry's lines with their common indent removed.
fn dedent(entry: &str) -> String {
    let indent = entry
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    entry
        .lines()
        .map(|l| l.get(indent..).unwrap_or(l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` with each of `needed` in its `permissions:`: added inside a flow
/// list on its line (the way the manifests are written), as items of a block
/// list, or as a new line at the end when there is no such key. Comments
/// stay; the permissions read afterwards are what they were plus what was
/// missing.
fn add_permissions(text: &str, needed: &[&str]) -> Result<String, String> {
    let parse = |t: &str| -> Result<Vec<String>, String> {
        let doc: serde_yaml::Value =
            serde_yaml::from_str(t).map_err(|e| format!("doesn't parse: {e}"))?;
        Ok(doc
            .get("permissions")
            .and_then(serde_yaml::Value::as_sequence)
            .map(|s| {
                s.iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    };
    let have = parse(text)?;
    let missing: Vec<&str> = needed
        .iter()
        .copied()
        .filter(|p| !have.iter().any(|h| h == p))
        .collect();
    if missing.is_empty() {
        return Ok(text.to_string());
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let at = lines.iter().position(|l| l.starts_with("permissions:"));
    let edited = match at {
        None => {
            let mut out = text.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("permissions: [{}]\n", missing.join(", ")));
            out
        }
        Some(at) if lines[at].contains('[') => {
            let line = lines[at];
            let close = line
                .find(']')
                .ok_or("`permissions:` is a flow list that continues on the next line; write it on one line")?;
            let open = line.find('[').unwrap_or(0);
            let empty = line[open + 1..close].trim().is_empty();
            let joined = missing.join(", ");
            let mut out: String = lines[..at].concat();
            out.push_str(&format!(
                "{}{}{}",
                &line[..close],
                if empty { joined } else { format!(", {joined}") },
                &line[close..]
            ));
            out.push_str(&lines[at + 1..].concat());
            out
        }
        Some(_) => {
            let mut out = text.to_string();
            for p in &missing {
                out = oxplow_config::append_list_entry(&out, "permissions", &format!("  - {p}\n"))
                    .map_err(|e| e.to_string())?;
            }
            out
        }
    };
    let mut expected = have;
    expected.extend(missing.iter().map(|m| m.to_string()));
    if parse(&edited)? != expected {
        return Err("`permissions` can't be edited in place; add them by hand".into());
    }
    Ok(edited)
}
