//! Which commands a person is offered where (`.context/commands.md` "When a
//! command is offered", `.context/extensions-slots.md` "Action points and
//! the store filter"). The command registry is one list, and a command with
//! a `ui` is offered under one of two keys:
//!
//! - an **item** key (`ui.about`, a resource kind): on a ref of that kind,
//!   narrowed by `ui.about.accepts.store` (the item URI's `store` param) and
//!   `ui.about_when` (a column of the ref's own row);
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

/// The key `ui` is offered under: its `about`, else its `at`; none for a
/// command search lists on its own.
pub fn key_of(ui: &CommandUi) -> Option<OfferKey> {
    match (&ui.about, &ui.at) {
        (Some(about), _) => Some(OfferKey::Item(about.resource().to_string())),
        (None, Some(point)) => Some(OfferKey::Point(point.clone())),
        (None, None) => None,
    }
}

/// Whether `ui`'s store filter lets an item with `params` through: some
/// entry of `accepts.store` is the item's `store` exactly, or holds for it
/// as a renderer's `accepts: { store: <rule> }` does
/// ([`crate::pages::renderers::takes`]). No filter: every item.
pub fn store_accepted(ui: &CommandUi, params: &BTreeMap<String, String>) -> bool {
    let Some(filter) = ui
        .about
        .as_ref()
        .and_then(|a| a.accepts())
        .and_then(|a| a.store.as_ref())
    else {
        return true;
    };
    let store = params.get("store");
    filter.rules().into_iter().any(|rule| {
        store.is_some_and(|s| s == rule)
            || crate::pages::renderers::takes(
                &BTreeMap::from([("store".to_string(), rule.to_string())]),
                params,
            )
    })
}

/// The commands of `specs` offered under `key`: for an item key, those of
/// the kind whose store filter lets an item with `params` through (their
/// `about_when` is tested on the ref's row, not here); for a point key,
/// those at the point.
pub fn offered<'a>(
    specs: &'a [CommandSpec],
    key: &OfferKey,
    params: &BTreeMap<String, String>,
) -> Vec<&'a CommandSpec> {
    specs
        .iter()
        .filter(|s| {
            let Some(ui) = &s.ui else { return false };
            key_of(ui).as_ref() == Some(key)
                && match key {
                    OfferKey::Item(_) => store_accepted(ui, params),
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
/// Not an oxplow URI: none.
pub async fn ref_commands(svc: &Services, uri: &str) -> Result<Vec<String>, String> {
    let Ok(parsed) = OxplowUri::parse(uri.trim()) else {
        return Ok(Vec::new());
    };
    let key = OfferKey::Item(parsed.resource().to_string());
    let all = svc.commands.list(&Actor::Human);
    let about = offered(&all, &key, parsed.params());
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
    ids.sort();
    Ok(ids)
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
        store
            .map(|s| ("store".to_string(), s.to_string()))
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
        assert_eq!(ids(offered(&specs, &item, &params(None))), ["a.item"]);
        assert_eq!(ids(offered(&specs, &point, &params(None))), ["a.point"]);
        assert_eq!(
            ids(offered(
                &specs,
                &OfferKey::Point("x/other".into()),
                &params(None)
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
            let mut found = ids(offered(&specs, &key, &params(store)));
            found.sort();
            found
        };
        assert_eq!(at(Some("oxtask")), ["any", "list", "one"]);
        assert_eq!(at(Some("gh")), ["any", "list"]);
        assert_eq!(at(Some("git-hub")), ["any", "pat"]);
        assert_eq!(at(Some("other")), ["any"]);
        assert_eq!(at(None), ["any", "none"]);
    }
}
