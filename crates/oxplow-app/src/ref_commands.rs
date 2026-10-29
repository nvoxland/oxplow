//! Which commands about a ref apply to it (`.context/commands.md` "When a
//! command is offered"): the commands that say they are about the ref's
//! kind (`ui.about`) and, for one with an `ui.about_when`, whose condition
//! holds on the ref's own row — the dataset row its resource resolves
//! through, read the way a resource page's `content` loader reads it.
//! The window's menus (a page's ☰, a row's right-click, the Navigator's
//! thread menu) ask once per ref and run no SQL themselves.

use oxplow_db::SqlCell;
use oxplow_domain::uri::OxplowUri;
use oxplow_domain::{Actor, CommandSpec};

use crate::pages::content::{resolve_content, ContentOpts};
use crate::viz_actions::when_holds;
use crate::Services;

/// The ids of the commands a person is offered about `uri`: those with
/// `ui.about` its resource, and a `ui.about_when` that holds on its row.
/// A command whose condition can't be tested (no row: the ref is gone, or
/// no dataset holds it) is not offered; one without a condition always is.
/// Not an oxplow URI: none.
pub async fn ref_commands(svc: &Services, uri: &str) -> Result<Vec<String>, String> {
    let Ok(parsed) = OxplowUri::parse(uri.trim()) else {
        return Ok(Vec::new());
    };
    let kind = parsed.resource().to_string();
    let about: Vec<CommandSpec> = svc
        .commands
        .list(&Actor::Human)
        .into_iter()
        .filter(|s| {
            s.ui.as_ref()
                .is_some_and(|ui| ui.about.as_deref() == Some(kind.as_str()))
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
        .map(|s| s.id)
        .collect();
    ids.sort();
    Ok(ids)
}
