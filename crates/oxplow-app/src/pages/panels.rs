//! A left-nav panel as one thing (`.context/extensions-page-host.md`
//! "Panels"): an id, title, icon and scope made of part pages — a `body`, and
//! optionally a `collapsed` line, a `count` page and a `badge` that feeds
//! Alerts. This is where what a panel means is decided once: which page each
//! part runs, with which params, the toggle a body's choice param gives, the
//! header's count and the alert. A fixture's `when: { panel }` and an agent
//! read a panel through [`run_panel`]. The window runs each part live
//! (`open_panel_part`) and every snapshot of a part's run carries its
//! [`PartView`]: what that run gives the panel, so the window keeps no rule.
//!
//! A part's scope id is never a param the caller writes: a stream panel's
//! pages declare `stream_id`, a thread panel's `thread_id`, and the run takes
//! it from the view (`ViewContext`, `resolve_param_values`), as every page does.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, DomainError};
use serde::{Deserialize, Serialize};

use crate::extension::extensions::ExtensionPanel;
use crate::pages::runner::{PageRun, PageRunner, RunRequest, RunTarget};
use crate::pages::schema::Page;
use crate::pages::spec::{AlertState, ChoiceOption, ViewContext};

/// One of a panel's parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum PanelPart {
    Body,
    Collapsed,
    Count,
    Badge,
}

impl PanelPart {
    pub const ALL: [PanelPart; 4] = [Self::Body, Self::Collapsed, Self::Count, Self::Badge];
}

/// The page `part` runs (`<extension>/<slug>`), when the panel has that part.
pub fn page_of(panel: &ExtensionPanel, part: PanelPart) -> Option<&str> {
    match part {
        PanelPart::Body => Some(&panel.body),
        PanelPart::Collapsed => panel.collapsed.as_deref(),
        PanelPart::Count => panel.count.as_deref(),
        PanelPart::Badge => panel.badge.as_deref(),
    }
}

/// A choice param of the body, as the panel's header toggle offers it: its
/// options and the value in force.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PanelChoice {
    pub name: String,
    pub label: String,
    pub options: Vec<ChoiceOption>,
    pub value: String,
}

/// The body's toggles: each param with fixed `options`, its value the
/// viewer's pick while the body still offers it, else the param's default,
/// else its first option. A pick the body dropped is not an error.
pub fn choices(body: &Page, picks: &BTreeMap<String, String>) -> Vec<PanelChoice> {
    body.params
        .iter()
        .filter(|p| !p.options.is_empty())
        .map(|p| {
            let offered = |v: &str| p.options.iter().any(|o| o.value == v);
            let default = match &p.default {
                Some(SqlCell::Text(t)) if offered(t) => t.clone(),
                _ => p.options[0].value.clone(),
            };
            let value = picks
                .get(&p.name)
                .filter(|v| offered(v))
                .cloned()
                .unwrap_or(default);
            PanelChoice {
                name: p.name.clone(),
                label: p.label.clone().unwrap_or_else(|| p.name.clone()),
                options: p.options.clone(),
                value,
            }
        })
        .collect()
}

/// A part's number: its `badge` when that is a number, else the rows of its
/// page's first loader.
pub fn part_number(run: &PageRun) -> Option<i64> {
    match &run.badge {
        Some(SqlCell::Int(n)) => return Some(*n),
        Some(SqlCell::Real(r)) if r.is_finite() => return Some(*r as i64),
        Some(SqlCell::Text(t)) => return t.trim().parse().ok(),
        _ => {}
    }
    let first = run.page.default_loader()?;
    Some(run.loaders.get(&first.name)?.result.rows.len() as i64)
}

/// The part the header counts by: the `count` page, else the badge. The
/// badge of a panel with a count page only alerts.
pub fn counting_part(panel: &ExtensionPanel) -> Option<PanelPart> {
    if panel.count.is_some() {
        Some(PanelPart::Count)
    } else if panel.badge.is_some() {
        Some(PanelPart::Badge)
    } else {
        None
    }
}

/// A badge's number for the header: a firing alert's count (a quiet one
/// says nothing), else the badge's value when it has no alert.
pub fn badge_count(alert: Option<&AlertState>, value: Option<i64>) -> Option<i64> {
    match alert {
        Some(alert) => alert.firing.then_some(alert.count),
        None => value,
    }
}

/// What one part's run gives its panel, as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PartView {
    /// The header counts by this part ([`counting_part`]).
    pub counts: bool,
    /// The header's count while it `counts`; none shows no count.
    #[specta(type = Option<oxplow_domain::JsNumber>)]
    pub count: Option<i64>,
    /// The body's toggles, each with the value it ran with ([`choices`]).
    pub choices: Vec<PanelChoice>,
}

/// What `run`, a run of `part` of `panel`, gives the panel.
pub fn part_view(panel: &ExtensionPanel, part: PanelPart, run: &PageRun) -> PartView {
    let counts = counting_part(panel) == Some(part);
    let count = match part {
        _ if !counts => None,
        PanelPart::Count => part_number(run),
        _ => badge_count(
            run.alert.as_ref(),
            run.badge.as_ref().and_then(|_| part_number(run)),
        ),
    };
    let choices = match part {
        PanelPart::Body => {
            let ran: BTreeMap<String, String> = run
                .params
                .iter()
                .filter_map(|(k, v)| match v {
                    SqlCell::Text(t) => Some((k.clone(), t.clone())),
                    _ => None,
                })
                .collect();
            self::choices(&run.page, &ran)
        }
        _ => Vec::new(),
    };
    PartView {
        counts,
        count,
        choices,
    }
}

/// A panel run whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PanelRun {
    pub panel: ExtensionPanel,
    pub body: PageRun,
    pub collapsed: Option<PageRun>,
    pub count_run: Option<PageRun>,
    pub badge: Option<PageRun>,
    /// The header's count: the counting part's ([`part_view`]).
    #[specta(type = Option<oxplow_domain::JsNumber>)]
    pub count: Option<i64>,
    /// The badge's alert, firing or not; a firing one is in Alerts.
    pub alert: Option<AlertState>,
    pub choices: Vec<PanelChoice>,
}

/// The loaded panel `id` (`<extension>/<panel>`).
pub async fn find_panel(svc: &crate::Services, id: &str) -> Result<ExtensionPanel, DomainError> {
    let extensions = svc.listed_extensions(svc.worktrees.project_dir()).await;
    let all: Vec<&ExtensionPanel> = extensions.iter().flat_map(|e| e.panels.iter()).collect();
    if let Some(panel) = all.iter().find(|p| p.id == id) {
        return Ok((*panel).clone());
    }
    let ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
    Err(DomainError::Invalid(format!(
        "no panel `{id}`; the loaded panels are {}",
        if ids.is_empty() {
            "none".into()
        } else {
            ids.join(", ")
        }
    )))
}

/// The params `part` runs with: the body takes the toggles' values in force;
/// every part's scope id comes from the view.
pub fn part_params(part: PanelPart, choices: &[PanelChoice]) -> BTreeMap<String, SqlCell> {
    match part {
        PanelPart::Body => choices
            .iter()
            .map(|c| (c.name.clone(), SqlCell::Text(c.value.clone())))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// The body's toggles for the viewer's `picks`, read from its page.
async fn body_choices(
    svc: &crate::Services,
    panel: &ExtensionPanel,
    picks: &BTreeMap<String, String>,
) -> Result<Vec<PanelChoice>, DomainError> {
    let body = svc
        .extension_catalog
        .find_page(svc.worktrees.project_dir(), &panel.body)?;
    Ok(choices(&body, picks))
}

/// What running `part` of `panel` for the view `ctx` is: its page, with
/// [`part_params`]. `None` when the panel has no such part. The one way a
/// part is run, by [`run_panel`] and by the window's live runs
/// (`open_panel_part`).
pub async fn part_request(
    svc: &crate::Services,
    panel: &ExtensionPanel,
    part: PanelPart,
    ctx: ViewContext,
    picks: &BTreeMap<String, String>,
) -> Result<Option<RunRequest>, DomainError> {
    let Some(page) = page_of(panel, part) else {
        return Ok(None);
    };
    let choices = match part {
        PanelPart::Body => body_choices(svc, panel, picks).await?,
        _ => Vec::new(),
    };
    Ok(Some(RunRequest {
        target: RunTarget::Page(page.into()),
        params: part_params(part, &choices),
        ctx,
        viewer: Actor::Human,
    }))
}

/// Run panel `id` for the view `ctx`, the body with the viewer's `picks`.
pub async fn run_panel(
    svc: &crate::Services,
    id: &str,
    ctx: ViewContext,
    picks: &BTreeMap<String, String>,
) -> Result<PanelRun, DomainError> {
    let panel = find_panel(svc, id).await?;
    let choices = body_choices(svc, &panel, picks).await?;
    let mut runs: BTreeMap<&str, PageRun> = BTreeMap::new();
    for part in PanelPart::ALL {
        let Some(req) = part_request(svc, &panel, part, ctx.clone(), picks).await? else {
            continue;
        };
        runs.insert(part_key(part), PageRunner::of(svc).run(req).await?);
    }
    let body = runs.remove("body").expect("a panel always has a body");
    let collapsed = runs.remove("collapsed");
    let count_run = runs.remove("count");
    let badge = runs.remove("badge");
    let count = counting_part(&panel).and_then(|part| {
        let run = match part {
            PanelPart::Count => count_run.as_ref(),
            _ => badge.as_ref(),
        }?;
        part_view(&panel, part, run).count
    });
    let alert = badge.as_ref().and_then(|b| b.alert.clone());
    Ok(PanelRun {
        panel,
        body,
        collapsed,
        count_run,
        badge,
        count,
        alert,
        choices,
    })
}

fn part_key(part: PanelPart) -> &'static str {
    match part {
        PanelPart::Body => "body",
        PanelPart::Collapsed => "collapsed",
        PanelPart::Count => "count",
        PanelPart::Badge => "badge",
    }
}

/// A panel as an agent reads it: its title and count, the toggles in force,
/// the alert when it fires, then the body as text.
pub async fn panel_text(svc: &crate::Services, run: &PanelRun) -> Result<String, DomainError> {
    let mut out = match run.count {
        Some(n) => format!("# {} ({n})\n", run.panel.title),
        None => format!("# {}\n", run.panel.title),
    };
    for c in &run.choices {
        out.push_str(&format!("{}: {}\n", c.label, c.value));
    }
    if let Some(alert) = run.alert.as_ref().filter(|a| a.firing) {
        out.push_str(&format!("Alert: {}\n", alert.message));
    }
    let body = crate::pages::text::page_text(svc, svc.worktrees.project_dir(), &run.body).await?;
    // The body's own title line repeats the panel's.
    let body = body.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
    out.push_str(body);
    Ok(out)
}
