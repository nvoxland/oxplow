//! Which commands a person is offered where (`.context/commands.md` "When a
//! command is offered", `.context/extensions-slots.md` "Action points and
//! the store filter"). The command registry is one list, and a command with
//! a `ui` is offered under one of two keys:
//!
//! - an **item** key (`ui.about`, a resource kind, or a web link's scheme):
//!   on a ref of that kind, narrowed by `ui.about.accepts.store` (the item
//!   URI's `store` param), `accepts.url` / `accepts.path` (a link's URL or
//!   path) and `ui.about_when` (a column of the ref's own row);
//! - a **point** key (`ui.at`, an action point's id): at a place a page
//!   puts that point, run with the point's context as its input.
//!
//! The window's menus (a page's ☰, a row's right-click, the Navigator's
//! thread menu, a page's action point) ask once per ref or point and run no
//! SQL themselves.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::uri::OxplowUri;
use oxplow_domain::{Actor, CommandSpec, CommandUi};

use crate::pages::content::{resolve_content, ContentOpts};
use crate::viz_actions::when_holds;
use crate::Services;

/// The key a command is offered under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferKey {
    /// On items of a resource kind (`oxplow/agent_session`).
    Item(String),
    /// At an action point (`oxplow/task.new`).
    Point(String),
}

/// Whether `ui` is offered under `key`: an item key by its `about`, a
/// point key by its `at` — a command with both is offered under both, as
/// one action.
pub fn has_key(ui: &CommandUi, key: &OfferKey) -> bool {
    match key {
        OfferKey::Item(resource) => ui.about.as_ref().is_some_and(|a| a.resource() == resource),
        OfferKey::Point(point) => ui.at.as_ref() == Some(point),
    }
}

/// The store filter of `ui` at a point: the `about`'s when it has one (the
/// one filter a command with both serves), else its own `accepts`.
pub fn point_accepts(ui: &CommandUi) -> Option<&oxplow_domain::commands::ItemAccepts> {
    ui.about
        .as_ref()
        .and_then(|a| a.accepts())
        .or(ui.accepts.as_ref())
}

/// Whether `ui`'s store filter lets an item with `params` through: some
/// entry of `accepts.store` is the item's store (the param `store_param`
/// names) exactly, or holds for it
/// as a renderer's `accepts: { store: <rule> }` does
/// ([`crate::pages::renderers::takes`]). No filter: every item.
pub fn store_accepted(
    ui: &CommandUi,
    params: &BTreeMap<String, String>,
    store_param: &str,
) -> bool {
    accepts_store(
        ui.about.as_ref().and_then(|a| a.accepts()),
        params,
        store_param,
    )
}

/// Whether an item with `params` passes `accepts`' store filter (none: it
/// does): an item command's (`about.accepts`) or a point command's
/// (`accepts`).
pub fn accepts_store(
    accepts: Option<&oxplow_domain::commands::ItemAccepts>,
    params: &BTreeMap<String, String>,
    store_param: &str,
) -> bool {
    let Some(filter) = accepts.and_then(|a| a.store.as_ref()) else {
        return true;
    };
    let store = params.get(store_param);
    filter.rules().into_iter().any(|rule| {
        store.is_some_and(|s| s == rule)
            || crate::pages::renderers::takes(
                &BTreeMap::from([("store".to_string(), rule.to_string())]),
                params,
            )
    })
}

/// Whether `ui`'s link filters let `uri` through: `about.accepts.url`, a
/// param pattern over the whole URL, and `about.accepts.path`, globs over
/// the item's `path` param (a file or directory's worktree path). None:
/// every link. A filter that doesn't compile lets nothing through (the
/// command's check refuses it before it registers).
pub fn link_accepted(ui: &CommandUi, uri: &str, params: &BTreeMap<String, String>) -> bool {
    let Some(accepts) = ui.about.as_ref().and_then(|a| a.accepts()) else {
        return true;
    };
    if let Some(pattern) = &accepts.url {
        let hit = crate::param_pattern::param_pattern(pattern)
            .ok()
            .and_then(|kept| regex::Regex::new(&kept).ok())
            .is_some_and(|re| re.is_match(uri.trim()));
        if !hit {
            return false;
        }
    }
    if let Some(globs) = &accepts.path {
        let Some(path) = params.get("path") else {
            return false;
        };
        let hit = globs.rules().into_iter().any(|glob| {
            globset::GlobBuilder::new(glob)
                .literal_separator(true)
                .build()
                .is_ok_and(|g| g.compile_matcher().is_match(path))
        });
        if !hit {
            return false;
        }
    }
    true
}

/// What is wrong with `ui`'s link filters, as `(field, message)`: a `url`
/// that isn't a param pattern, a `path` that isn't a glob. Checked where an
/// extension's command registers, beside `CommandUi::problem`.
pub fn link_filter_problem(ui: &CommandUi) -> Option<(String, String)> {
    let accepts = ui.about.as_ref().and_then(|a| a.accepts())?;
    if let Some(pattern) = &accepts.url {
        if let Err(why) = crate::param_pattern::param_pattern(pattern) {
            return Some((
                "/ui/about/accepts/url".into(),
                format!("`{pattern}` isn't a URL pattern: {why}"),
            ));
        }
    }
    for glob in accepts.path.iter().flat_map(|p| p.rules()) {
        if let Err(why) = globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
        {
            return Some((
                "/ui/about/accepts/path".into(),
                format!("`{glob}` isn't a glob: {why}"),
            ));
        }
    }
    None
}

/// The param that names an item's store: the one its shared model declares
/// (`store_param`) when a model's resource is `resource`, else `store`.
pub fn store_param_of(registry: &crate::models::ModelRegistry, resource: &str) -> String {
    registry
        .models
        .values()
        .find(|m| m.decl.resource == resource)
        .map(|m| m.decl.store_param.clone())
        .unwrap_or_else(|| "store".to_string())
}

/// The commands of `specs` offered under `key`: for an item key, those of
/// the kind whose store filter lets an item with `params` through (their
/// `about_when` is tested on the ref's row, not here); for a point key,
/// those at the point.
pub fn offered<'a>(
    specs: &'a [CommandSpec],
    key: &OfferKey,
    params: &BTreeMap<String, String>,
    store_param: &str,
) -> Vec<&'a CommandSpec> {
    specs
        .iter()
        .filter(|s| {
            let Some(ui) = &s.ui else { return false };
            has_key(ui, key)
                && match key {
                    OfferKey::Item(_) => store_accepted(ui, params, store_param),
                    OfferKey::Point(_) => true,
                }
        })
        .collect()
}

/// The ids of the commands a person is offered about `uri`: those with
/// `ui.about` its resource, a `ui.about.accepts.store` its `store` param passes,
/// and a `ui.about_when` that holds on its row.
/// A command whose condition can't be tested (no row: the ref is gone, or
/// no dataset holds it) is not offered; one without a condition always is.
/// A web URL is named by its scheme (`about: https`), as a page that draws
/// it is (`renderers::resource_of`); a link filter (`accepts.url`,
/// `accepts.path`) narrows either. Anything else: none.
pub async fn ref_commands(svc: &Services, uri: &str) -> Result<Vec<String>, String> {
    let Some(resource) = crate::pages::renderers::resource_of(uri) else {
        return Ok(Vec::new());
    };
    let params = OxplowUri::parse(uri.trim())
        .map(|u| u.params().clone())
        .unwrap_or_default();
    let key = OfferKey::Item(resource.clone());
    let all = svc.commands.list(&Actor::Human);
    let store_param = store_param_of(&svc.models.current(), &resource);
    let about: Vec<&CommandSpec> = offered(&all, &key, &params, &store_param)
        .into_iter()
        .filter(|s| {
            s.ui.as_ref()
                .is_some_and(|ui| link_accepted(ui, uri, &params))
        })
        .collect();
    // One read of the row, and only when some command's offer depends on it.
    let conditional = about
        .iter()
        .any(|s| s.ui.as_ref().is_some_and(|ui| ui.about_when.is_some()));
    let row = if conditional {
        let opts = ContentOpts {
            text: false,
            max_bytes: 0,
        };
        let found = resolve_content(svc, uri, None, opts).await?;
        found.rows.into_iter().next().map(|cells| {
            found
                .columns
                .into_iter()
                .zip(cells)
                .collect::<Vec<(String, SqlCell)>>()
        })
    } else {
        None
    };
    let mut ids: Vec<String> = about
        .into_iter()
        .filter(|s| {
            let when = s.ui.as_ref().and_then(|ui| ui.about_when.as_deref());
            match (when, &row) {
                (None, _) => true,
                (Some(column), Some(row)) => row
                    .iter()
                    .find(|(name, _)| name == column)
                    .is_some_and(|(_, cell)| when_holds(cell)),
                (Some(_), None) => false,
            }
        })
        .map(|s| s.id.clone())
        .collect();
    // `about_if`: the command's own script decides on this link, all of
    // the menu's at once. Boxed: its script runs' state is large, and a
    // menu's ref commands are awaited deep in a fixture's or a page's run.
    let refused = Box::pin(refused_by_about_if(
        svc, &all, &ids, uri, &resource, &params,
    ))
    .await;
    ids.retain(|id| !refused.contains(id));
    ids.sort();
    Ok(ids)
}

/// The ids among `ids` whose command's `about_if` says no to `uri`.
async fn refused_by_about_if(
    svc: &Services,
    all: &[CommandSpec],
    ids: &[String],
    uri: &str,
    resource: &str,
    params: &BTreeMap<String, String>,
) -> Vec<String> {
    let deciding = about_ifs(all, ids);
    if deciding.is_empty() {
        return Vec::new();
    }
    let input = about_if_input(uri, resource, params);
    let decided = futures::future::join_all(deciding.iter().map(|(id, func)| {
        Box::pin(about_if_holds(
            svc,
            id.clone(),
            func.clone(),
            uri.to_string(),
            input.clone(),
        ))
    }))
    .await;
    deciding
        .into_iter()
        .zip(decided)
        .filter(|(_, holds)| !holds)
        .map(|((id, _), _)| id)
        .collect()
}

/// The ids among `ids` whose command has a `ui.about_if`, with its function.
fn about_ifs(all: &[CommandSpec], ids: &[String]) -> Vec<(String, String)> {
    all.iter()
        .filter(|s| ids.contains(&s.id))
        .filter_map(|s| {
            let func = s.ui.as_ref()?.about_if.clone()?;
            Some((s.id.clone(), func))
        })
        .collect()
}

/// What an `about_if` function is called with: the link, its kind, and
/// the URL (a web link) or the worktree path (a file or directory).
pub fn about_if_input(
    uri: &str,
    resource: &str,
    params: &BTreeMap<String, String>,
) -> serde_json::Value {
    let mut input = serde_json::json!({ "uri": uri.trim(), "kind": resource });
    if oxplow_domain::commands::is_web_resource(resource) {
        input["url"] = serde_json::Value::String(uri.trim().to_string());
    } else if let Some(path) = params.get("path") {
        input["path"] = serde_json::Value::String(path.clone());
    }
    input
}

/// How long one `about_if` call may run: a menu waits on it.
const ABOUT_IF_BUDGET: oxplow_domain::run::Budget =
    oxplow_domain::run::Budget::with_timeout(std::time::Duration::from_secs(1));

/// How long an `about_if` answer is kept for the same command, link and
/// script: a menu opened again soon asks no script.
const ABOUT_IF_KEPT: std::time::Duration = std::time::Duration::from_secs(30);

type AboutIfKey = (String, String, u64);

fn about_if_answers(
) -> &'static std::sync::Mutex<std::collections::HashMap<AboutIfKey, (std::time::Instant, bool)>> {
    static ANSWERS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<AboutIfKey, (std::time::Instant, bool)>>,
    > = std::sync::OnceLock::new();
    ANSWERS.get_or_init(Default::default)
}

/// The script of command `id` (an enabled extension's `entry:`) and the
/// services it needs.
fn command_script(svc: &Services, id: &str) -> Option<(String, Vec<String>)> {
    svc.extension_catalog
        .get(&svc.layout.project_dir)
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.commands.iter())
        .find(|c| c.name == id)
        .and_then(|c| match &c.handler {
            crate::extension::extension_commands::CommandHandler::Script { script, .. } => {
                Some((script.clone(), c.needs.clone()))
            }
            _ => None,
        })
}

/// Whether command `id`'s `func` returns `True` for the link: one script
/// run (`ABOUT_IF_BUDGET`), with the services the command needs (its
/// `sql.read` answers a query). A run that fails, runs too long or answers
/// anything else is no, and is logged.
async fn about_if_holds(
    svc: &Services,
    id: String,
    func: String,
    uri: String,
    input: serde_json::Value,
) -> bool {
    use std::hash::{Hash, Hasher};
    let Some((script, needs)) = command_script(svc, &id) else {
        return false;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    script.hash(&mut hasher);
    let key = (id.clone(), uri, hasher.finish());
    if let Some((at, holds)) = about_if_answers()
        .lock()
        .ok()
        .and_then(|m| m.get(&key).copied())
    {
        if at.elapsed() < ABOUT_IF_KEPT {
            return holds;
        }
    }
    // As the person opening the menu, with the command's own services: a
    // `sql.read` it needs answers its `scope("sql.read", …)` here.
    let run = oxplow_domain::run::RunContext::core(Actor::Human, oxplow_domain::run::Mode::Pure);
    let languages = svc.languages.current();
    let given = match crate::services::decide(&run, &script, &needs)
        .and_then(|wanted| wanted.scopes_only(languages.clone()))
    {
        Ok(given) => given,
        Err(e) => {
            tracing::warn!(command = %id, error = %e, "a command's about_if can't have its services: not offered");
            return false;
        }
    };
    let (layer, runtime) = (svc.sql.clone(), tokio::runtime::Handle::current());
    let ran = tokio::task::spawn_blocking(move || {
        use oxplow_script::workers::{shared, ScriptRun};
        let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(layer.run(q)));
        let mut calls =
            crate::scope_calls::Calls::new(&run, &needs, read).with_languages(languages);
        let mut serve = |scope: &str, args: serde_json::Value| calls.serve(scope, args);
        shared().run(
            &ABOUT_IF_BUDGET,
            given.onto(
                ScriptRun::starlark_fn(&script, &func, input),
                Some(&mut serve),
            ),
            Some(run.cancel()),
        )
    })
    .await;
    let holds = match ran {
        Ok(Ok(ran)) => ran.value == serde_json::Value::Bool(true),
        Ok(Err(e)) => {
            tracing::warn!(command = %id, error = %e, "a command's about_if failed: not offered");
            false
        }
        Err(e) => {
            tracing::warn!(command = %id, error = %e, "a command's about_if didn't finish: not offered");
            false
        }
    };
    if let Ok(mut answers) = about_if_answers().lock() {
        answers.retain(|_, (at, _)| at.elapsed() < ABOUT_IF_KEPT);
        answers.insert(key, (std::time::Instant::now(), holds));
    }
    holds
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{Access, Atomicity, Confirm, Lifecycle};
    use serde_json::json;

    fn spec(id: &str, ui: serde_json::Value) -> CommandSpec {
        CommandSpec {
            output_schema: None,
            id: id.into(),
            summary: "s".into(),
            input_schema: json!({}),
            job: false,
            confirm: Confirm::Never,
            confirm_says: None,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::External,
            access: Access::Write,
            needs: vec![],
            ui: Some(serde_json::from_value(ui).unwrap()),
            op: None,
            unrecorded: vec![],
        }
    }

    fn ids(found: Vec<&CommandSpec>) -> Vec<&str> {
        found.into_iter().map(|s| s.id.as_str()).collect()
    }

    fn params(store: Option<&str>) -> BTreeMap<String, String> {
        params_named("store", store)
    }

    fn params_named(name: &str, store: Option<&str>) -> BTreeMap<String, String> {
        store
            .map(|s| (name.to_string(), s.to_string()))
            .into_iter()
            .collect()
    }

    /// One list, two kinds of key: an item command is under its kind, a
    /// point command under its point, and neither shows under the other.
    #[test]
    fn item_and_point_commands_are_one_list_under_two_keys() {
        let specs = vec![
            spec("a.item", json!({ "label": "A", "about": "x/thing" })),
            spec("a.point", json!({ "label": "B", "at": "x/thing.new" })),
            spec("a.search", json!({ "label": "C" })),
        ];
        let item = OfferKey::Item("x/thing".into());
        let point = OfferKey::Point("x/thing.new".into());
        assert_eq!(
            ids(offered(&specs, &item, &params(None), "store")),
            ["a.item"]
        );
        assert_eq!(
            ids(offered(&specs, &point, &params(None), "store")),
            ["a.point"]
        );
        assert_eq!(
            ids(offered(
                &specs,
                &OfferKey::Point("x/other".into()),
                &params(None),
                "store"
            )),
            Vec::<&str>::new()
        );
    }

    /// The store filter is the renderers' `accepts` match on the item's
    /// `store` param: a store or a list of them, a rule (`present`, an
    /// anchored pattern); an item with no store passes only no filter or
    /// `absent`.
    #[test]
    fn a_store_filter_matches_the_items_store_param() {
        let specs = vec![
            spec("any", json!({ "label": "Any", "about": "x/t" })),
            spec(
                "one",
                json!({ "label": "One", "about": { "resource": "x/t", "accepts": { "store": "oxtask" } } }),
            ),
            spec(
                "list",
                json!({ "label": "List", "about": { "resource": "x/t", "accepts": { "store": ["gh", "oxtask"] } } }),
            ),
            spec(
                "pat",
                json!({ "label": "Pat", "about": { "resource": "x/t", "accepts": { "store": "^git-[a-z]+$" } } }),
            ),
            spec(
                "none",
                json!({ "label": "None", "about": { "resource": "x/t", "accepts": { "store": "absent" } } }),
            ),
        ];
        let key = OfferKey::Item("x/t".into());
        let at = |store| {
            let mut found = ids(offered(&specs, &key, &params(store), "store"));
            found.sort();
            found
        };
        assert_eq!(at(Some("oxtask")), ["any", "list", "one"]);
        assert_eq!(at(Some("gh")), ["any", "list"]);
        assert_eq!(at(Some("git-hub")), ["any", "pat"]);
        assert_eq!(at(Some("other")), ["any"]);
        assert_eq!(at(None), ["any", "none"]);
    }

    /// A command about web links or files narrows them by the URL (a
    /// pattern over it whole) or the worktree path (globs, `*` stopping at
    /// `/`); with neither, every link of its kind.
    #[test]
    fn a_link_filter_matches_the_url_or_the_path() {
        let gh = spec(
            "gh",
            json!({ "label": "PR", "about": { "resource": "https", "accepts": { "url": "^https://github[.]com/[a-z]+/[a-z]+/pull/[0-9]+$" } } }),
        );
        let any_web = spec("web", json!({ "label": "Web", "about": "https" }));
        let nb = spec(
            "nb",
            json!({ "label": "Jupyter", "about": { "resource": "oxplow/file", "accepts": { "path": ["**/*.ipynb", "*.ipynb"] } } }),
        );
        let ui = |s: &CommandSpec| s.ui.clone().unwrap();
        let none = BTreeMap::new();
        let pr = "https://github.com/acme/app/pull/12";
        assert!(link_accepted(&ui(&gh), pr, &none));
        assert!(!link_accepted(
            &ui(&gh),
            "https://github.com/acme/app",
            &none
        ));
        assert!(link_accepted(&ui(&any_web), "https://example.com", &none));
        let at = |path: &str| BTreeMap::from([("path".to_string(), path.to_string())]);
        assert!(link_accepted(
            &ui(&nb),
            "oxplow:oxplow/file?path=a.ipynb",
            &at("a.ipynb")
        ));
        assert!(link_accepted(
            &ui(&nb),
            "oxplow:oxplow/file?path=x/y/a.ipynb",
            &at("x/y/a.ipynb")
        ));
        assert!(!link_accepted(
            &ui(&nb),
            "oxplow:oxplow/file?path=a.py",
            &at("a.py")
        ));
        assert!(!link_accepted(&ui(&nb), "oxplow:oxplow/file", &none));
    }

    /// A URL filter is a param pattern and a path filter a glob, or the
    /// command is refused where it registers.
    #[test]
    fn a_link_filter_that_cant_match_is_refused() {
        let ui = |v| -> CommandUi { serde_json::from_value(v).unwrap() };
        assert_eq!(
            link_filter_problem(&ui(
                json!({ "label": "x", "about": { "resource": "https", "accepts": { "url": "^https://a[.]b/[a-z/]+$" } } })
            )),
            None
        );
        let (field, _) = link_filter_problem(&ui(json!({ "label": "x", "about": { "resource": "https", "accepts": { "url": "github.com" } } })))
            .expect("unanchored");
        assert_eq!(field, "/ui/about/accepts/url");
        let (field, _) = link_filter_problem(&ui(json!({ "label": "x", "about": { "resource": "oxplow/file", "accepts": { "path": "a/[b" } } })))
            .expect("bad glob");
        assert_eq!(field, "/ui/about/accepts/path");
    }

    /// A shared model may name its store by another param (tasks keep
    /// `provider`): the filter reads that one, and the name comes from the
    /// model whose resource the item is, else `store`.
    #[test]
    fn a_store_filter_reads_the_param_the_model_names_its_store_by() {
        let specs = vec![spec(
            "one",
            json!({ "label": "One", "about": { "resource": "x/t", "accepts": { "store": "oxtask" } } }),
        )];
        let key = OfferKey::Item("x/t".into());
        let by_provider = params_named("provider", Some("oxtask"));
        assert_eq!(
            ids(offered(&specs, &key, &by_provider, "provider")),
            ["one"]
        );
        assert!(offered(&specs, &key, &by_provider, "store").is_empty());

        let mut registry = crate::models::ModelRegistry::default();
        registry.models.insert(
            "x/t".into(),
            crate::models::ModelSpec {
                decl: oxplow_db::model_store::ModelDecl {
                    extension: "x".into(),
                    namespace: "x".into(),
                    name: "t".into(),
                    resource: "x/t".into(),
                    key: "id".into(),
                    store_param: "provider".into(),
                    search: None,
                    columns: vec![],
                    children: vec![],
                },
                open: true,
                change_event: None,
            },
        );
        assert_eq!(store_param_of(&registry, "x/t"), "provider");
        assert_eq!(store_param_of(&registry, "x/other"), "store");
    }
}
