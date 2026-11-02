//! Which pages can draw a URI, decided here and nowhere else
//! (`.context/pages-and-tabs.md` "Which page draws a URI"). The desktop
//! host, Open With, an agent's `oxplow.page.renderers` and `find`'s
//! `about` all ask [`renderers_for`].
//!
//! A page is offered for a URI when
//!
//! 1. its `for` names the URI's resource (an oxplow URI's
//!    `<extension>/<name>`, a web page's scheme),
//! 2. its `accepts` hold for the URI's params — each named param `present`,
//!    `absent` or matching its pattern, and
//! 3. when it says `accepts: { content: <loader> }`, that one loader of
//!    its own yields at least one row. Only that loader and what it reads
//!    run (`sql`, `rows`, a read-only `command`, the system's `content`);
//!    2 seconds, 1 MiB; a failure of any kind means not offered.
//!
//! The order is the extensions' (and each one's pages'), pages that open
//! the file to decide (their check reads `content`) after those that
//! don't, a check on the URI's params alone among the latter; `default` is
//! the page's own flag, and the caller picks: the URI's view
//! (`#oxplow.renderer=`), else the default, else the first.

use std::collections::BTreeMap;
use std::time::Duration;

use oxplow_db::{SqlCell, SqlQueryResult};
use oxplow_domain::uri::OxplowUri;
use oxplow_domain::Actor;
use serde::{Deserialize, Serialize};

use super::content::{resolve_content, ContentOpts};
use super::runner::PageRunner;
use super::schema::{Loader, Page, CONTENT};
use super::spec::ViewContext;
use crate::extension::extensions::RENDERED_SCHEMES;
use crate::Services;

/// The longest the daemon spends deciding whether one page accepts a URI.
pub const ACCEPT_TIMEOUT: Duration = Duration::from_secs(2);

/// A page that can draw a URI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Renderer {
    /// `<extension>/<slug>`; what a view's `oxplow.renderer` names.
    pub page: String,
    pub title: String,
    /// The page's own `default` flag.
    pub default: bool,
}

/// What a URI is drawn by: an oxplow URI's resource, an http(s) URL's
/// scheme; `None` for anything else.
pub fn resource_of(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if let Ok(parsed) = OxplowUri::parse(uri) {
        return Some(parsed.resource().to_string());
    }
    let scheme = uri.split_once("://")?.0.to_ascii_lowercase();
    let plain = scheme
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    (plain && RENDERED_SCHEMES.contains(&scheme.as_str())).then_some(scheme)
}

/// The `?` params of a URI (none for a web page's).
fn params_of(uri: &str) -> BTreeMap<String, String> {
    OxplowUri::parse(uri.trim())
        .map(|u| u.params().clone())
        .unwrap_or_default()
}

/// Whether `accepts` hold for `params`: each named param present, absent,
/// or matching its pattern (a whole-value match, patterns being anchored).
pub fn takes(accepts: &BTreeMap<String, String>, params: &BTreeMap<String, String>) -> bool {
    accepts.iter().all(|(key, rule)| {
        let value = params.get(key);
        match rule.as_str() {
            "present" => value.is_some(),
            "absent" => value.is_none(),
            pattern => value.is_some_and(|v| {
                crate::param_pattern::param_pattern(pattern)
                    .ok()
                    .and_then(|kept| regex::Regex::new(&kept).ok())
                    .is_some_and(|re| re.is_match(v))
            }),
        }
    })
}

/// The pages of `pages` whose `for` names `uri`'s resource and whose
/// `accepts` hold for its params, in order. Its content isn't looked at.
pub fn candidates<'p>(pages: &'p [Page], uri: &str) -> Vec<&'p Page> {
    let Some(resource) = resource_of(uri) else {
        return Vec::new();
    };
    let params = params_of(uri);
    pages
        .iter()
        .filter(|p| p.renders.contains(&resource) && takes(&p.accepts, &params))
        .collect()
}

/// Every enabled extension's pages, in the order the extensions list them.
fn enabled_pages(svc: &Services) -> Vec<Page> {
    svc.extension_catalog
        .unpinned(&svc.layout.project_dir, "renderers read one snapshot")
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.page_files.iter().cloned())
        .collect()
}

/// The pages that can draw `uri` for `viewer` viewing from `ctx`.
pub async fn renderers_for(
    svc: &Services,
    uri: &str,
    ctx: &ViewContext,
    viewer: &Actor,
) -> Vec<Renderer> {
    renderers_in(svc, &enabled_pages(svc), uri, ctx, viewer, ACCEPT_TIMEOUT).await
}

/// [`renderers_for`] among `pages`, each content check given `timeout`.
pub(crate) async fn renderers_in(
    svc: &Services,
    pages: &[Page],
    uri: &str,
    ctx: &ViewContext,
    viewer: &Actor,
    timeout: Duration,
) -> Vec<Renderer> {
    let survivors = candidates(pages, uri);
    let content = shared_content(svc, &survivors, uri, ctx, timeout).await;
    let runner = PageRunner::of(svc);
    let verdicts =
        futures::future::join_all(survivors.iter().map(|page| {
            accepts_content(&runner, page, uri, content.as_ref(), ctx, viewer, timeout)
        }))
        .await;
    let mut offered: Vec<&Page> = survivors
        .into_iter()
        .zip(verdicts)
        .filter(|(_, offered)| *offered)
        .map(|(page, _)| page)
        .collect();
    // A page that opens the file to decide is a special view of what the
    // others draw whole: it follows them, so the first (when none is
    // flagged `default`) is the general one. A check on the URI's params
    // alone is general. Stable: the extensions' order holds within each
    // kind.
    offered.sort_by_key(|page| reads_content(page));
    offered
        .into_iter()
        .map(|page| Renderer {
            page: page.id.clone(),
            title: page.title.clone(),
            default: page.default,
        })
        .collect()
}

/// How `page`'s `content` loader reads.
fn content_opts(page: &Page) -> Option<ContentOpts> {
    page.loaders.iter().find_map(|l| match &l.loader {
        Loader::Content { text, max_bytes } => Some(ContentOpts {
            text: *text,
            max_bytes: *max_bytes,
        }),
        _ => None,
    })
}

/// Whether deciding on `page` reads the `content` loader.
fn reads_content(page: &Page) -> bool {
    page.accepts_content
        .as_deref()
        .is_some_and(|l| page.loader_chain(l).iter().any(|n| n == CONTENT))
}

/// The record `uri` names, resolved once for every surviving page that
/// reads it to decide: with text only if one of them asks for it (the
/// largest `max_bytes` among them). `None` when none reads it.
async fn shared_content(
    svc: &Services,
    survivors: &[&Page],
    uri: &str,
    ctx: &ViewContext,
    timeout: Duration,
) -> Option<Result<SqlQueryResult, String>> {
    let asks: Vec<ContentOpts> = survivors
        .iter()
        .filter(|p| reads_content(p))
        .filter_map(|p| content_opts(p))
        .collect();
    if asks.is_empty() {
        return None;
    }
    let opts = ContentOpts {
        text: asks.iter().any(|o| o.text),
        max_bytes: asks.iter().map(|o| o.max_bytes).max().unwrap_or_default(),
    };
    let answered = tokio::time::timeout(timeout, resolve_content(svc, uri, ctx.stream_id, opts))
        .await
        .unwrap_or_else(|_| Err("reading the content took too long".into()));
    Some(answered)
}

/// The params `page`'s check is run with: the URI itself (its identity,
/// without the view) and those of the URI's params the page declares.
fn supplied(page: &Page, uri: &str) -> BTreeMap<String, SqlCell> {
    let identity = OxplowUri::parse(uri.trim())
        .map(|u| u.identity())
        .unwrap_or_else(|_| uri.trim().to_string());
    let from_uri = params_of(uri);
    let mut out: BTreeMap<String, SqlCell> = page
        .params
        .iter()
        .filter_map(|p| from_uri.get(&p.name).map(|v| (p.name.clone(), v.clone())))
        .map(|(k, v)| (k, SqlCell::Text(v)))
        .collect();
    out.insert("uri".into(), SqlCell::Text(identity));
    out
}

/// Whether `page` passes its `accepts: { content: … }` (none: it does).
async fn accepts_content(
    runner: &PageRunner<'_>,
    page: &Page,
    uri: &str,
    content: Option<&Result<SqlQueryResult, String>>,
    ctx: &ViewContext,
    viewer: &Actor,
    timeout: Duration,
) -> bool {
    let Some(loader) = page.accepts_content.as_deref() else {
        return true;
    };
    let content = if reads_content(page) {
        match content {
            Some(Ok(result)) => Some(result.clone()),
            _ => return false,
        }
    } else {
        None
    };
    let asked = runner.run_loader_chain(page, loader, supplied(page, uri), ctx, viewer, content);
    matches!(
        tokio::time::timeout(timeout, asked).await,
        Ok(Ok(result)) if !result.rows.is_empty()
    )
}
