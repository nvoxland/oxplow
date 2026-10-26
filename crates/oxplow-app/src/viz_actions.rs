//! Running a lens's actions (P6.B1, `.context/extensions.md`): each is a
//! command the lens offers, run through the bus as `Actor::Lens` acting for
//! whoever pressed it — a person from the UI, the calling agent from MCP.
//! The bus applies that actor's whole policy (the agent policy,
//! confirmation), so a lens offers a button and grants no power of its
//! own: it runs as its presser would, and can't confirm on anyone's
//! behalf.

use std::collections::BTreeMap;
use std::path::Path;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, CommandError, CommandOutcome};
use serde_json::Value;

use crate::extensions;

/// Where a dragged row landed: the rows on either side of the drop
/// (`before`: the one below it, `after`: the one above), the row it was
/// dropped on, and the value of the group it was dropped in. What isn't
/// there is `None` (a drop at a list's end has no row below). `order` is
/// every row of the result in its order after the drop (the dragged rows
/// moved to where they landed; a drop onto a row leaves it as it was).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Drop {
    #[serde(default)]
    pub before: Option<BTreeMap<String, SqlCell>>,
    #[serde(default)]
    pub after: Option<BTreeMap<String, SqlCell>>,
    #[serde(default)]
    pub onto: Option<BTreeMap<String, SqlCell>>,
    #[serde(default)]
    pub group: Option<SqlCell>,
    #[serde(default)]
    pub order: Option<Vec<BTreeMap<String, SqlCell>>>,
}

impl Drop {
    /// The value `{{scope.name}}` names here, if any: a neighbour row's
    /// column, or the group's value. `{{order.*}}` is a list, not a cell
    /// ([`Drop::order_of`]).
    fn value(&self, scope: &str, name: &str) -> Option<SqlCell> {
        let row = match scope {
            "before" => self.before.as_ref(),
            "after" => self.after.as_ref(),
            "onto" => self.onto.as_ref(),
            "group" if name == "value" => return self.group.clone(),
            _ => return None,
        };
        row?.get(name).cloned()
    }

    /// `{{order.name}}`: that column of every row in the new order, if the
    /// drop has an order and every row has the column.
    fn order_of(&self, name: &str) -> Option<Value> {
        self.order
            .as_ref()?
            .iter()
            .map(|r| r.get(name).and_then(|c| serde_json::to_value(c).ok()))
            .collect::<Option<Vec<_>>>()
            .map(Value::Array)
    }
}

/// The scopes a drop binds.
const DROP_SCOPES: &[&str] = &["before", "after", "onto", "group", "order"];

/// The command a lens's action, form or component runs, resolved from one
/// catalog generation. It runs after that generation is let go: a command
/// that activates (an extension's deploy, enable, disable or remove) waits
/// for every reader of the generation it replaces, and would wait on its
/// own.
pub(crate) struct Prepared {
    actor: Actor,
    command: String,
    input: Value,
    confirmed: bool,
}

impl Prepared {
    pub(crate) async fn run(self, svc: &crate::Services) -> Result<CommandOutcome, CommandError> {
        svc.commands
            .run(&self.actor, &self.command, self.input, self.confirmed)
            .await
    }
}

/// The command `action_id` declared in `actions`, its input bound from
/// resolved `params`, `row` and `drop`, acting as `actor_id` for
/// `on_behalf_of`. `source_id` identifies the declaration in diagnostics;
/// neither a query nor a visualization config is needed to prepare it.
/// Row/drop/when/form checks are shared by page and lens actions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_resolved(
    source_id: &str,
    actions: &[extensions::PageAction],
    actor_id: &str,
    action_id: &str,
    params: &BTreeMap<String, SqlCell>,
    row: Option<&BTreeMap<String, SqlCell>>,
    given: Option<Value>,
    drop: Option<Drop>,
    on_behalf_of: Actor,
    confirmed: bool,
) -> Result<Prepared, CommandError> {
    let invalid = |field: &str, message: String| CommandError::Invalid {
        field: Some(field.into()),
        message,
    };
    let action = actions
        .iter()
        .find(|a| a.id == action_id)
        .cloned()
        .ok_or_else(|| {
            let ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
            invalid(
                "/action",
                format!("lens `{source_id}` has no action `{action_id}` (it has {ids:?})"),
            )
        })?;
    match (action.row || action.one_row, row) {
        (true, None) => {
            return Err(invalid(
                "/row",
                format!("`{action_id}` is a row action; it runs on a row"),
            ))
        }
        (false, Some(_)) => {
            return Err(invalid(
                "/row",
                format!("`{action_id}` runs on the whole lens, not a row"),
            ))
        }
        _ => {}
    }
    match (action.drag, &drop) {
        (true, None) => {
            return Err(invalid(
                "/drop",
                format!("`{action_id}` runs on a drop: say where the row landed"),
            ))
        }
        (false, Some(_)) => {
            return Err(invalid(
                "/drop",
                format!("`{action_id}` isn't a drag action (`drag: true`)"),
            ))
        }
        _ => {}
    }
    if let (Some(when), Some(row)) = (&action.when, &row) {
        let truthy = match row.get(when) {
            None | Some(SqlCell::Null(())) => false,
            Some(SqlCell::Int(n)) => *n != 0,
            Some(SqlCell::Real(n)) => *n != 0.0,
            Some(SqlCell::Bool(b)) => *b,
            Some(SqlCell::Text(t)) => !t.is_empty(),
        };
        if !truthy {
            return Err(invalid(
                "/row",
                format!("`{action_id}` isn't offered on this row (`when: {when}`)"),
            ));
        }
    }
    let bound = bind_input(&action.input, params, row, drop.as_ref())?;
    let input = match given {
        None => bound,
        Some(_) if !action.form => {
            return Err(invalid(
                "/input",
                format!(
                    "`{action_id}` runs as declared; only a form action (`form: true`) takes input"
                ),
            ))
        }
        Some(given) => filled(bound, given, &action.fixed)?,
    };
    Ok(Prepared {
        actor: Actor::Lens {
            lens_id: actor_id.to_string(),
            on_behalf_of: Box::new(on_behalf_of),
        },
        command: action.command,
        input,
        confirmed,
    })
}

/// What a form shows: its command (the fields come from its input
/// schema), the values the fields start from, the fields shown but not
/// editable, and which fields to show (none: all).
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct FormStart {
    pub command: oxplow_domain::CommandSpec,
    #[specta(type = oxplow_domain::Json)]
    pub values: Value,
    pub fixed: Vec<String>,
    pub fields: Option<Vec<crate::extensions::FormField>>,
    /// Fields picked from a list, with their choices.
    pub choices: BTreeMap<String, Vec<Choice>>,
    /// Object fields' children, read from their `from` lens.
    pub subfields: BTreeMap<String, Vec<Subfield>>,
    /// What the `from` lenses read: when it changes, the form is read
    /// again.
    pub reads: oxplow_db::Reads,
}

/// A child of an object field (a row of its `from` lens).
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct Subfield {
    pub name: String,
    pub title: String,
    /// `enum`, `text` or `number`.
    pub kind: String,
    /// An `enum`'s values.
    pub values: Vec<String>,
    /// Set by the source, shown by nothing here: left out of the form.
    pub read_only: bool,
}

/// An object field's children from the rows its `from` source returned:
/// `name`, `title`, `kind`, `values` (a JSON array) and `read_only`.
pub(crate) fn subfields_from(
    result: &oxplow_db::SqlQueryResult,
    field: &str,
    source: &str,
) -> Result<Vec<Subfield>, CommandError> {
    let col = |name: &str| result.columns.iter().position(|c| c == name);
    let (Some(name), Some(title), Some(kind)) = (col("name"), col("title"), col("kind")) else {
        return Err(CommandError::Invalid {
            field: Some(format!("/{field}")),
            message: format!("`from` source `{source}` needs `name`, `title` and `kind` columns"),
        });
    };
    let (values, read_only) = (col("values"), col("read_only"));
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => Some(t.clone()),
        _ => None,
    };
    Ok(result
        .rows
        .iter()
        .filter_map(|r| {
            Some(Subfield {
                name: text(&r[name])?,
                title: text(&r[title])?,
                kind: text(&r[kind])?,
                values: values
                    .and_then(|v| text(&r[v]))
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default(),
                read_only: read_only.is_some_and(|c| matches!(&r[c], SqlCell::Int(n) if *n != 0)),
            })
        })
        .collect())
}

/// A field's choices from the rows its source returned: `value`, `label`
/// and an optional `group`.
pub(crate) fn choices_from(
    result: &oxplow_db::SqlQueryResult,
    field: &str,
    source: &str,
) -> Result<Vec<Choice>, CommandError> {
    let col = |name: &str| result.columns.iter().position(|c| c == name);
    let (Some(value), Some(label)) = (col("value"), col("label")) else {
        return Err(CommandError::Invalid {
            field: Some(format!("/{field}")),
            message: format!("choices source `{source}` needs `value` and `label` columns"),
        });
    };
    let group = col("group");
    let text = |c: &SqlCell| match c {
        SqlCell::Text(t) => Some(t.clone()),
        SqlCell::Int(i) => Some(i.to_string()),
        SqlCell::Real(r) => Some(r.to_string()),
        _ => None,
    };
    Ok(result
        .rows
        .iter()
        .filter_map(|r| {
            Some(Choice {
                value: text(&r[value])?,
                label: text(&r[label])?,
                group: group.and_then(|g| text(&r[g])),
            })
        })
        .collect())
}

/// One choice for a form field: the value it submits, what the person
/// reads, and the heading it sits under.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub group: Option<String>,
}

/// `given` (what a person filled in, a map) laid over `start`, with the
/// `fixed` fields put back to `start`'s values: what a form runs with.
pub(crate) fn filled(start: Value, given: Value, fixed: &[String]) -> Result<Value, CommandError> {
    let Value::Object(given) = given else {
        return Err(CommandError::Invalid {
            field: Some("/input".into()),
            message: "a form's input is its fields, a map".into(),
        });
    };
    let mut out = match start {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let kept: Vec<(String, Value)> = fixed
        .iter()
        .filter_map(|k| out.get(k).map(|v| (k.clone(), v.clone())))
        .collect();
    out.extend(given);
    out.extend(kept);
    Ok(Value::Object(out))
}

/// The custom component `key` names (`<extension>/<component id>`) — the
/// one a frame shows, whether a lens or a page shows it — from its
/// extension.
fn component_of(
    svc: &crate::Services,
    lens_root: &Path,
    key: &str,
) -> Result<
    (
        extensions::Extension,
        extensions::custom_components::CustomComponent,
    ),
    CommandError,
> {
    let (ext_name, id) = key
        .split_once('/')
        .ok_or_else(|| CommandError::from(oxplow_domain::DomainError::NotFound))?;
    let ext = svc
        .extension_catalog
        .named(lens_root, ext_name)
        .map_err(CommandError::from)?;
    let component = ext
        .custom_components
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or_else(|| CommandError::Invalid {
            field: Some("/id".into()),
            message: format!("`{ext_name}` has no custom component `{id}`"),
        })?;
    Ok((ext, component))
}

/// The form of one of component `key`'s commands, for the host to show
/// when the frame asks (`openForm`): the command's schema, starting from
/// `input`. Submitting it runs through [`invoke_component_command`], as
/// the component.
pub fn component_form(
    svc: &crate::Services,
    lens_root: &Path,
    key: &str,
    command: &str,
    input: Value,
    version: &str,
) -> Result<FormStart, CommandError> {
    let (ext, component) = component_of(svc, lens_root, key)?;
    let loaded = svc
        .component_bundles
        .get(version)
        .filter(|bundle| {
            bundle.extension == ext.name
                && bundle.component == component.id
                && svc.component_bundles.is_current(lens_root, bundle)
        })
        .ok_or_else(|| CommandError::Denied {
            reason: format!(
                "component `{}/{}` isn't loaded at that version any more; reload it",
                ext.name, component.id
            ),
        })?;
    if !loaded.commands.iter().any(|c| c == command) {
        return Err(CommandError::Invalid {
            field: Some("/command".into()),
            message: format!(
                "`{command}` isn't one of component `{}`'s commands ({})",
                component.id,
                loaded.commands.join(", ")
            ),
        });
    }
    let spec = svc
        .commands
        .spec(command)
        .ok_or_else(|| CommandError::Unknown {
            name: command.to_string(),
        })?;
    Ok(FormStart {
        command: spec,
        values: match input {
            Value::Null => Value::Object(serde_json::Map::new()),
            other => other,
        },
        fixed: Vec::new(),
        fields: None,
        choices: BTreeMap::new(),
        subfields: BTreeMap::new(),
        reads: oxplow_db::Reads::default(),
    })
}

/// [`component_form`] under one pinned extension catalog generation.
pub async fn component_form_pinned(
    svc: &crate::Services,
    lens_root: &Path,
    key: &str,
    command: &str,
    input: Value,
    version: &str,
) -> Result<FormStart, CommandError> {
    svc.sql
        .read_generation(async { component_form(svc, lens_root, key, command, input, version) })
        .await
        .map_err(CommandError::from)?
}

/// A custom component runs its own read-only SQL (`querySql`).
pub struct ComponentSql {
    /// The component whose frame asked: `<extension>/<component id>`.
    pub component: String,
    pub sql: String,
    /// Positional (`?1`, `?2`).
    pub params: Vec<SqlCell>,
    /// The stream the frame is shown in, for the live models.
    pub stream: Option<i64>,
    /// Row cap: 1,000 by default, at most 10,000.
    pub limit: Option<usize>,
    /// Who is looking at the frame.
    pub on_behalf_of: Actor,
    /// The bundle version the frame was loaded at.
    pub version: String,
}

/// Run `call`'s SQL for a component that declares `sql: read`, at the
/// version loaded now: read-only over the published models, as the
/// component's run under its extension's permissions (`permissions.sql`),
/// so the read is decided and recorded like a script's `sql.read`.
pub async fn run_component_sql(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentSql,
) -> Result<oxplow_db::SqlQueryResult, CommandError> {
    let (run, query) = svc
        .sql
        .read_generation(async { prepare_component_sql(svc, lens_root, call) })
        .await
        .map_err(CommandError::from)??;
    svc.sql
        .run_in(&run, query)
        .await
        .map_err(CommandError::from)
}

fn prepare_component_sql(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentSql,
) -> Result<(oxplow_domain::run::RunContext, oxplow_db::SqlQuery), CommandError> {
    let (ext, component) = component_of(svc, lens_root, &call.component)?;
    let loaded = svc
        .component_bundles
        .get(&call.version)
        .filter(|bundle| {
            bundle.extension == ext.name
                && bundle.component == component.id
                && svc.component_bundles.is_current(lens_root, bundle)
        })
        .ok_or_else(|| CommandError::Denied {
            reason: format!(
                "component `{}/{}` isn't loaded at that version any more; reload it",
                ext.name, component.id
            ),
        })?;
    if !loaded.sql {
        return Err(CommandError::Invalid {
            field: Some("/sql".into()),
            message: format!(
                "component `{}` doesn't declare `sql: read`; add it (and `permissions.sql: read`) \
                 to run its own SQL",
                component.id
            ),
        });
    }
    let run = oxplow_domain::run::RunContext::contribution(
        crate::run_context::contribution(
            &svc.extension_catalog,
            lens_root,
            &ext,
            oxplow_domain::contribution::ContributionKind::CustomComponent,
            &component.id,
        ),
        Actor::Component {
            component: call.component,
            on_behalf_of: Box::new(call.on_behalf_of),
        },
        oxplow_domain::run::Mode::Pure,
    )
    .authorized(crate::grants::authority(
        &ext,
        crate::grants::script_request(&["sql.read".to_string()]),
    ));
    if let oxplow_domain::grant::Decision::Denied(why) = run.decide_service("sql.read") {
        return Err(CommandError::Denied { reason: why });
    }
    let mut query = oxplow_db::SqlQuery::new(call.sql)
        .positional(call.params)
        .stream(call.stream)
        .limit(Some(call.limit.unwrap_or(1_000).min(10_000)));
    crate::grants::bound_query(&run, &mut query);
    Ok((run, query))
}

/// A custom component invokes one of its declared commands.
pub struct ComponentInvoke {
    /// The component whose frame asked: `<extension>/<component id>`.
    pub component: String,
    pub command: String,
    /// Sent as is: a component's input is literal (no placeholders).
    pub input: Value,
    /// Who is looking at the frame.
    pub on_behalf_of: Actor,
    /// The person confirmed this call, in the host (never in the frame).
    pub confirmed: bool,
    /// The bundle version the frame was loaded at
    /// ([`crate::component_bundles`]): what is running, and what must be
    /// approved.
    pub version: String,
}

/// Run `call`'s command as the component acting for its viewer, so every
/// policy applies as if they ran it — a component can offer an action but
/// never grant a power — when the bundle its frame was loaded at declares
/// the command and a person approved that version (tsk960, tsk984): what
/// acts is what was approved, whatever the disk says now.
pub async fn invoke_component_command(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentInvoke,
) -> Result<CommandOutcome, CommandError> {
    svc.sql
        .read_generation(async { prepare_component_command(svc, lens_root, call) })
        .await
        .map_err(CommandError::from)??
        .run(svc)
        .await
}

fn prepare_component_command(
    svc: &crate::Services,
    lens_root: &Path,
    call: ComponentInvoke,
) -> Result<Prepared, CommandError> {
    let (ext, component) = component_of(svc, lens_root, &call.component)?;
    let loaded = svc
        .component_bundles
        .get(&call.version)
        .filter(|bundle| {
            bundle.extension == ext.name
                && bundle.component == component.id
                && svc.component_bundles.is_current(lens_root, bundle)
        })
        .ok_or_else(|| CommandError::Denied {
            reason: format!(
                "component `{}/{}` isn't loaded at that version any more; reload its lens",
                ext.name, component.id
            ),
        })?;
    if !loaded.commands.contains(&call.command) {
        return Err(CommandError::Invalid {
            field: Some("/command".into()),
            message: format!(
                "`{}` isn't one of component `{}`'s commands ({})",
                call.command,
                component.id,
                loaded.commands.join(", ")
            ),
        });
    }
    Ok(Prepared {
        actor: Actor::Component {
            component: call.component,
            on_behalf_of: Box::new(call.on_behalf_of),
        },
        command: call.command,
        input: call.input,
        confirmed: call.confirmed,
    })
}

/// Load the bundle of component `key` (`<extension>/<id>`) as it is now
/// under `lens_root`, for its frame: the version to serve it at and invoke
/// with.
pub fn load_component(
    svc: &crate::Services,
    lens_root: &Path,
    key: &str,
) -> Result<String, CommandError> {
    let (ext, component) = component_of(svc, lens_root, key)?;
    let loaded = svc
        .component_bundles
        .load(lens_root, &ext, &component)
        .map_err(CommandError::from)?;
    Ok(loaded.version.clone())
}

/// Load a component under one pinned extension catalog generation.
pub async fn load_component_pinned(
    svc: &crate::Services,
    lens_root: &Path,
    key: &str,
) -> Result<String, CommandError> {
    svc.sql
        .read_generation(async { load_component(svc, lens_root, key) })
        .await
        .map_err(CommandError::from)?
}

/// `input` with its placeholders bound: a string that is exactly
/// `{{param.x}}` / `{{row.x}}` becomes that value (typed), `{{row.x|json}}`
/// the JSON that text cell holds (an array a row built), `{{param.x.id}}`
/// the key of the URI param `x` (a bare id), `{{order.x}}` the list of a
/// drop's rows' `x` in their new order (only as a field's whole value),
/// and a string that contains them has them spliced in as text. The values are data — they
/// go into the command's input, never into SQL.
pub fn bind_input(
    input: &Value,
    params: &BTreeMap<String, SqlCell>,
    row: Option<&BTreeMap<String, SqlCell>>,
    drop: Option<&Drop>,
) -> Result<Value, CommandError> {
    let lookup = |scope: &str, name: &str| -> Result<SqlCell, CommandError> {
        let found = match scope {
            "param" => params.get(name).cloned().or_else(|| {
                // `x.id`: the key of the URI param `x`.
                let SqlCell::Text(uri) = params.get(name.strip_suffix(".id")?)? else {
                    return None;
                };
                let u = oxplow_domain::uri::oxplow_uri::OxplowUri::parse(uri).ok()?;
                let key = oxplow_domain::uri::build::key_param(&u.resource().to_string())?;
                u.param(&key).map(|k| SqlCell::Text(k.to_string()))
            }),
            "row" => row.and_then(|r| r.get(name)).cloned(),
            "order" => {
                return Err(CommandError::Invalid {
                    field: Some("/input".into()),
                    message: format!(
                        "`{{{{order.{name}}}}}` is a list: bind it as a field's whole value"
                    ),
                })
            }
            s if DROP_SCOPES.contains(&s) => drop.and_then(|d| d.value(s, name)),
            _ => None,
        };
        found.ok_or_else(|| CommandError::Invalid {
            field: Some("/input".into()),
            message: format!("`{{{{{scope}.{name}}}}}` has no value here"),
        })
    };
    let text = |c: &SqlCell| match c {
        SqlCell::Null(()) => String::new(),
        SqlCell::Text(t) => t.clone(),
        SqlCell::Int(i) => i.to_string(),
        SqlCell::Real(r) => r.to_string(),
        SqlCell::Bool(b) => b.to_string(),
    };
    Ok(match input {
        Value::String(s) => match oxplow_domain::template::whole_placeholder(s) {
            Some(p) if p.filter.as_deref() == Some("json") => match lookup(&p.scope, &p.name)? {
                SqlCell::Text(t) => {
                    serde_json::from_str(&t).map_err(|e| CommandError::Invalid {
                        field: Some("/input".into()),
                        message: format!("`{{{{{}.{}|json}}}}` isn't JSON: {e}", p.scope, p.name),
                    })?
                }
                other => serde_json::to_value(other).unwrap_or(Value::Null),
            },
            Some(p) if p.scope == "order" => {
                drop.and_then(|d| d.order_of(&p.name))
                    .ok_or_else(|| CommandError::Invalid {
                        field: Some("/input".into()),
                        message: format!("`{{{{order.{}}}}}` has no value here", p.name),
                    })?
            }
            Some(p) => serde_json::to_value(lookup(&p.scope, &p.name)?).unwrap_or(Value::Null),
            None => Value::String(oxplow_domain::template::splice(s, |p| {
                lookup(&p.scope, &p.name).map(|c| text(&c))
            })?),
        },
        Value::Array(items) => {
            // A list is the whole value `{{order.x}}` binds, never an item.
            if let Some(p) = items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(oxplow_domain::template::whole_placeholder)
                .find(|p| p.scope == "order")
            {
                return Err(CommandError::Invalid {
                    field: Some("/input".into()),
                    message: format!(
                        "`{{{{order.{}}}}}` is a list: bind it as a field's whole value",
                        p.name
                    ),
                });
            }
            Value::Array(
                items
                    .iter()
                    .map(|i| bind_input(i, params, row, drop))
                    .collect::<Result<_, _>>()?,
            )
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                // A field that is exactly a drop's place with nothing there
                // (no row below a drop at the end) is left out: the
                // command's optional field stays absent. A drop always has
                // its order.
                let absent = v
                    .as_str()
                    .and_then(oxplow_domain::template::whole_placeholder)
                    .is_some_and(|p| {
                        p.scope != "order"
                            && DROP_SCOPES.contains(&p.scope.as_str())
                            && drop.is_none_or(|d| d.value(&p.scope, &p.name).is_none())
                    });
                if !absent {
                    out.insert(k.clone(), bind_input(v, params, row, drop)?);
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_tasks::work_item_ref;
    use serde_json::json;

    /// A task, and a lens over it whose actions run commands.
    async fn fixture() -> (crate::test_fixtures::TaskEffortFixture, std::path::PathBuf) {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("lenses")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/tasks.yaml"),
            r#"title: Tasks
query: "SELECT uri, title FROM v_work_item"
params: [{ name: item, default: "" }]
actions:
  - { id: finish, label: Finish, command: oxplow.work_item.transition, input: { uri: "{{param.item}}", to: done } }
  - { id: finish-row, label: Finish, command: oxplow.work_item.transition, row: true, input: { uri: "{{row.uri}}", to: done } }
  - { id: finish-open, label: Finish, command: oxplow.work_item.transition, row: true, when: open, input: { uri: "{{row.uri}}", to: done } }
  - { id: push, label: Push, command: oxplow.vcs.push, input: { stream: str1 } }
  - { id: prompt, label: Prompt, command: oxplow.config.set, input: { key: agentPromptAppend, value: be brief } }
  - { id: retitle, label: Retitle…, command: oxplow.work_item.update, row: true, form: true, fixed: [uri], input: { uri: "{{row.uri}}", title: "{{row.title}}" } }
"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/tiles.yaml"),
            r#"title: Tiles
params: [{ name: id, default: "" }]
query: |
  SELECT 'dti' || i.id AS id, i.kind,
         json_extract(i.options_json, '$.title') AS title,
         json_extract(i.options_json, '$.size') AS size,
         json_extract(i.options_json, '$.lensId') AS lens,
         json_extract(i.options_json, '$.sql') AS sql,
         json_extract(i.options_json, '$.display') AS display,
         json_extract(i.options_json, '$.chart') AS chart,
         json_extract(i.options_json, '$.metric') AS metric,
         json_extract(i.options_json, '$.viz') AS viz,
         json_extract(i.options_json, '$.text') AS text
  FROM v_dashboard_item i WHERE 'dsh' || i.dashboard_id = :id ORDER BY i.sort_index
viz: dashboard
tile: { id: id, title: title, size: size, lens: lens, sql: sql, display: display, chart: chart, metric: metric, viz: viz, text: text }
actions:
  - { id: reorder, label: Reorder, command: oxplow.dashboard.reorder_items, drag: true, input: { dashboard: "{{param.id}}", order: "{{order.id}}" } }
  - { id: rename, label: Rename…, command: oxplow.dashboard.update_item, row: true, form: true, fixed: [item], input: { item: "{{row.id}}", title: "{{row.title}}", size: "{{row.size}}" } }
  - { id: edit-text, label: Save, command: oxplow.dashboard.update_item, edit: true, fixed: [item], input: { item: "{{row.id}}", text: "{{row.text}}" } }
  - { id: remove, label: Remove, command: oxplow.dashboard.remove_item, row: true, key: Delete, input: { item: "{{row.id}}" } }
  - { id: add-text, label: Add Heading…, command: oxplow.dashboard.add_item, form: true, fixed: [dashboard, kind], input: { dashboard: "{{param.id}}", kind: text } }
  - { id: rename-dashboard, label: Rename Dashboard…, command: oxplow.dashboard.rename, form: true, fixed: [dashboard], input: { dashboard: "{{param.id}}" } }
"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/one.yaml"),
            r#"title: One
params: [{ name: n, default: "1" }]
query: |
  SELECT 'Made' AS title, CAST(:n AS INTEGER) = 1 AS open WHERE CAST(:n AS INTEGER) <> 0
  UNION ALL SELECT 'Twin', 1 WHERE CAST(:n AS INTEGER) = 2
viz: detail
columns: [{ key: title }, { key: open }]
actions:
  - { id: make, label: Make, command: oxplow.dashboard.create, when: open, input: { title: "{{row.title}}" } }
  - { id: plain, label: Plain, command: oxplow.dashboard.create, input: { title: Plain } }
  - { id: rename, label: Rename, command: oxplow.dashboard.rename, form: true, fixed: [dashboard], input: { dashboard: dsh1, title: "{{row.title}}" } }
"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/board.yaml"),
            r#"title: Board
query: "SELECT uri, title, state FROM v_work_item"
viz: board
group: { by: state, values: [todo, in_progress, done] }
actions:
  - { id: move, label: Move, command: oxplow.work_item.transition, drag: true, input: { uri: "{{row.uri}}", to: "{{group.value}}" } }
  - { id: reorder, label: Reorder, command: oxplow.work_item.reorder, drag: true, input: { uri: "{{row.uri}}", before: "{{before.uri}}", after: "{{after.uri}}" } }
  - { id: finish, label: Finish, command: oxplow.work_item.transition, row: true, input: { uri: "{{row.uri}}", to: done } }
"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("lenses/note.yaml"),
            r#"title: Note
query: "SELECT uri, body FROM v_work_item"
viz: markdown
markdown: { text: body }
actions:
  - { id: save, label: Save, command: oxplow.work_item.update, edit: true, fixed: [uri], input: { uri: "{{row.uri}}", body: "{{row.body}}" } }
"#,
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        (fx, root)
    }

    fn call(
        action: &str,
        params: BTreeMap<String, SqlCell>,
        row: Option<BTreeMap<String, SqlCell>>,
        on_behalf_of: Actor,
        confirmed: bool,
    ) -> LensActionCall {
        LensActionCall {
            lens_id: "acme/tasks".into(),
            action_id: action.into(),
            params,
            row,
            input: None,
            drop: None,
            on_behalf_of,
            confirmed,
        }
    }

    async fn audit_actor(svc: &crate::Services) -> (String, Option<String>) {
        svc.db
            .read(|c| {
                c.query_row(
                    "SELECT actor_kind, actor_id FROM command_audit ORDER BY id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap()
    }

    async fn state(svc: &crate::Services, task: oxplow_tasks::TaskId) -> String {
        use oxplow_tasks::TaskStore as _;
        let t = svc.task_store.get(task).await.unwrap().unwrap();
        serde_json::to_value(t.status)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    /// P6.B1's red: a person's click runs the command as the lens, acting
    /// for them — audited as `oxplow:oxplow/lens?lens=<id>`.
    #[tokio::test]
    async fn an_action_runs_its_command_as_the_lens() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        run_lens_action(
            &fx.svc,
            &root,
            call(
                "finish",
                BTreeMap::from([("item".to_string(), SqlCell::Text(item))]),
                None,
                Actor::Human,
                false,
            ),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
        let (kind, _) = audit_actor(&fx.svc).await;
        assert_eq!(kind, "lens");
    }

    /// A row action binds the row it was invoked on; a whole-lens action
    /// refuses a row and a row action needs one.
    #[tokio::test]
    async fn a_row_action_binds_its_row() {
        let (fx, root) = fixture().await;
        let row = BTreeMap::from([(
            "uri".to_string(),
            SqlCell::Text(oxplow_tasks::work_item_ref(fx.task)),
        )]);
        let run = |action: &'static str, row: Option<BTreeMap<String, SqlCell>>| {
            let (svc, root) = (fx.svc.clone(), root.clone());
            async move {
                run_lens_action(
                    &svc,
                    &root,
                    call(action, BTreeMap::new(), row, Actor::Human, false),
                    &extensions::ViewContext::default(),
                )
                .await
            }
        };
        assert!(run("finish-row", None).await.is_err());
        assert!(run("finish", Some(row.clone())).await.is_err());
        run("finish-row", Some(row)).await.unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A drop runs the drag action with where the row landed: the group's
    /// value, and the neighbours — one that isn't there (a drop at the end)
    /// is left out of the input, not sent empty. A drop on an action that
    /// isn't a drag one, or a drag action without a drop, is refused.
    #[tokio::test]
    async fn a_drop_runs_the_drag_action_with_where_the_row_landed() {
        let (fx, root) = fixture().await;
        let row = BTreeMap::from([(
            "uri".to_string(),
            SqlCell::Text(oxplow_tasks::work_item_ref(fx.task)),
        )]);
        let run = |action: &'static str, drop: Option<Drop>| {
            let (svc, root, row) = (fx.svc.clone(), root.clone(), row.clone());
            async move {
                let mut c = call(action, BTreeMap::new(), Some(row), Actor::Human, false);
                c.lens_id = "acme/board".into();
                c.drop = drop;
                run_lens_action(&svc, &root, c, &extensions::ViewContext::default()).await
            }
        };
        run(
            "move",
            Some(Drop {
                group: Some(SqlCell::Text("in_progress".into())),
                ..Drop::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "in_progress");

        // Dropped at the end of its list: no row below or above it.
        run("reorder", Some(Drop::default())).await.unwrap();

        let err = run("finish", Some(Drop::default())).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/drop"),
            "{err:?}"
        );
        let err = run("move", None).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), message } if f == "/drop" && message.contains("runs on a drop")),
            "{err:?}"
        );
    }

    /// A markdown editor's Save is its edit action: the person's text
    /// fills the edited field and the fixed `uri` keeps its row's value.
    #[tokio::test]
    async fn an_editors_save_runs_its_edit_action_with_the_text() {
        let (fx, root) = fixture().await;
        let item = oxplow_tasks::work_item_ref(fx.task);
        let row = BTreeMap::from([
            ("uri".to_string(), SqlCell::Text(item.clone())),
            ("body".to_string(), SqlCell::Text("Old body".into())),
        ]);
        let mut press = call("save", BTreeMap::new(), Some(row), Actor::Human, false);
        press.lens_id = "acme/note".into();
        press.input = Some(json!({
            "uri": "oxplow:oxplow/work_item?id=tsk999999&provider=oxplow",
            "body": "## New\n\nThe saved text."
        }));
        run_lens_action(&fx.svc, &root, press, &extensions::ViewContext::default())
            .await
            .unwrap();
        let body: String = fx
            .svc
            .db
            .read(move |c| {
                c.query_row("SELECT body FROM v_work_item WHERE uri = ?1", [item], |r| {
                    r.get(0)
                })
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(
            body, "## New\n\nThe saved text.",
            "the fixed uri kept this item"
        );
    }

    /// A whole-lens action over a lens that holds one row binds
    /// `{{row.*}}` from that row, which the server reads from the lens
    /// itself: offered by `when`, refused unless the lens has exactly one
    /// row, and refused a row the caller makes up.
    #[tokio::test]
    async fn a_whole_lens_action_over_a_one_row_lens_binds_that_row() {
        let (fx, root) = fixture().await;
        let svc = fx.svc.clone();
        let press = |action: &str, n: &str, row: Option<BTreeMap<String, SqlCell>>| {
            let (svc, root) = (svc.clone(), root.clone());
            let call = LensActionCall {
                lens_id: "acme/one".into(),
                action_id: action.into(),
                params: BTreeMap::from([("n".to_string(), SqlCell::Text(n.into()))]),
                row,
                input: None,
                drop: None,
                on_behalf_of: Actor::Human,
                confirmed: false,
            };
            async move { run_lens_action(&svc, &root, call, &extensions::ViewContext::default()).await }
        };
        let titles = || async {
            svc.sql
                .query_sql("SELECT title FROM v_dashboard ORDER BY id", vec![], None)
                .await
                .unwrap()
                .rows
                .into_iter()
                .map(|r| r[0].clone())
                .collect::<Vec<_>>()
        };
        press("make", "1", None).await.unwrap();
        assert_eq!(titles().await, vec![SqlCell::Text("Made".into())]);

        // Not offered: its `when` column is false on the one row.
        let off = press("make", "3", None).await.unwrap_err().to_string();
        assert!(off.contains("isn't offered"), "{off}");
        // Not one row.
        for (n, says) in [("0", "the lens has 0 rows"), ("2", "the lens has 2 rows")] {
            let err = press("make", n, None).await.unwrap_err().to_string();
            assert!(
                err.contains(says) && err.contains("needs one"),
                "{n}: {err}"
            );
        }
        // A row the caller makes up is refused, not trusted.
        let forged = BTreeMap::from([
            ("title".to_string(), SqlCell::Text("Forged".into())),
            ("open".to_string(), SqlCell::Int(1)),
        ]);
        let err = press("make", "1", Some(forged))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("whole lens"), "{err}");
        // A form over it starts from the row.
        let form = action_form(
            &svc,
            &root,
            LensActionCall {
                lens_id: "acme/one".into(),
                action_id: "rename".into(),
                params: BTreeMap::new(),
                row: None,
                input: None,
                drop: None,
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(form.values["title"], "Made");
        // An ordinary button still runs on any lens.
        press("plain", "2", None).await.unwrap();
        assert_eq!(titles().await.len(), 2);
    }

    /// A dashboard lens whose rows are its tiles edits them through its
    /// actions alone: a drag binds the whole new order, row forms rename
    /// and resize, the edit action saves a heading, a key removes, and
    /// whole-lens forms add a tile and rename the dashboard.
    #[tokio::test]
    async fn a_dashboard_lens_reorders_renames_and_removes_its_tiles() {
        let (fx, root) = fixture().await;
        let svc = fx.svc.clone();
        let made = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.dashboard.create",
                json!({ "title": "D" }),
                false,
            )
            .await
            .unwrap();
        let dashboard = made.result["id"].as_str().unwrap().to_string();
        for text in ["A", "B", "C"] {
            svc.commands
                .run(
                    &Actor::Human,
                    "oxplow.dashboard.add_item",
                    json!({ "dashboard": dashboard, "kind": "text", "text": text }),
                    false,
                )
                .await
                .unwrap();
        }
        let params = BTreeMap::from([("id".to_string(), SqlCell::Text(dashboard.clone()))]);
        // The tiles as the lens reads them: each row by its column.
        let tiles = || {
            let (svc, root, params) = (svc.clone(), root.clone(), params.clone());
            async move {
                let ctx = extensions::ViewContext::default();
                let run = extensions::run_lens(
                    &svc.sql,
                    &svc.extension_catalog,
                    &root,
                    "acme/tiles",
                    params,
                    &ctx,
                )
                .await
                .unwrap();
                let cols = run.result.columns.clone();
                run.result
                    .rows
                    .into_iter()
                    .map(|r| {
                        cols.iter()
                            .cloned()
                            .zip(r)
                            .collect::<BTreeMap<String, SqlCell>>()
                    })
                    .collect::<Vec<_>>()
            }
        };
        let texts = |rows: &[BTreeMap<String, SqlCell>], col: &str| {
            rows.iter()
                .map(|r| match &r[col] {
                    SqlCell::Text(t) => t.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
        };
        let act = |action: &'static str,
                   row: Option<BTreeMap<String, SqlCell>>,
                   input: Option<Value>,
                   drop: Option<Drop>| {
            let (svc, root, params) = (svc.clone(), root.clone(), params.clone());
            async move {
                let mut c = call(action, params, row, Actor::Human, false);
                c.lens_id = "acme/tiles".into();
                c.input = input;
                c.drop = drop;
                run_lens_action(&svc, &root, c, &extensions::ViewContext::default()).await
            }
        };

        // C dragged to the front: the drop's order is every tile.
        let rows = tiles().await;
        assert_eq!(texts(&rows, "text"), ["A", "B", "C"]);
        let order = vec![rows[2].clone(), rows[0].clone(), rows[1].clone()];
        act(
            "reorder",
            Some(rows[2].clone()),
            None,
            Some(Drop {
                before: Some(rows[0].clone()),
                order: Some(order),
                ..Drop::default()
            }),
        )
        .await
        .unwrap();
        let rows = tiles().await;
        assert_eq!(texts(&rows, "text"), ["C", "A", "B"]);

        // Rename and resize A; edit C's heading in place.
        act(
            "rename",
            Some(rows[1].clone()),
            Some(json!({ "title": "First", "size": "wide" })),
            None,
        )
        .await
        .unwrap();
        act(
            "edit-text",
            Some(rows[0].clone()),
            Some(json!({ "text": "Sea" })),
            None,
        )
        .await
        .unwrap();
        let rows = tiles().await;
        assert_eq!(texts(&rows, "title"), ["", "First", ""]);
        assert_eq!(texts(&rows, "size"), ["", "wide", ""]);
        assert_eq!(texts(&rows, "text"), ["Sea", "A", "B"]);

        // Remove B; add a heading; rename the dashboard.
        act("remove", Some(rows[2].clone()), None, None)
            .await
            .unwrap();
        act("add-text", None, Some(json!({ "text": "New" })), None)
            .await
            .unwrap();
        act(
            "rename-dashboard",
            None,
            Some(json!({ "title": "Board" })),
            None,
        )
        .await
        .unwrap();
        let rows = tiles().await;
        assert_eq!(texts(&rows, "text"), ["Sea", "A", "New"]);
        let title = svc
            .sql
            .query_sql(
                "SELECT title FROM v_dashboard WHERE 'dsh' || id = ?1",
                vec![SqlCell::Text(dashboard.clone())],
                None,
            )
            .await
            .unwrap();
        assert_eq!(title.rows, vec![vec![SqlCell::Text("Board".into())]]);
    }

    /// Where a drop landed binds `{{before.*}}`, `{{after.*}}`, `{{onto.*}}`
    /// and `{{group.value}}`; a field that is exactly one of them with
    /// nothing there is left out; anywhere else, it has no value.
    #[test]
    fn a_drops_bindings_fill_its_placeholders_and_leave_out_what_isnt_there() {
        let near = |uri: &str| BTreeMap::from([("uri".to_string(), SqlCell::Text(uri.into()))]);
        let drop = Drop {
            before: Some(near("b")),
            after: None,
            onto: Some(near("o")),
            group: Some(SqlCell::Text("done".into())),
            order: Some(vec![near("x"), near("b"), near("y")]),
        };
        let input = serde_json::json!({
            "before": "{{before.uri}}",
            "after": "{{after.uri}}",
            "parent": "{{onto.uri}}",
            "to": "{{group.value}}",
            "note": "near {{before.uri}}",
            "order": "{{order.uri}}"
        });
        assert_eq!(
            bind_input(&input, &BTreeMap::new(), None, Some(&drop)).unwrap(),
            serde_json::json!({
                "before": "b", "parent": "o", "to": "done", "note": "near b",
                "order": ["x", "b", "y"]
            })
        );
        for bad in [
            serde_json::json!({ "list": ["{{after.uri}}"] }),
            serde_json::json!({ "note": "near {{after.uri}}" }),
            serde_json::json!({ "list": ["{{order.uri}}"] }),
            serde_json::json!({ "note": "in {{order.uri}}" }),
            serde_json::json!({ "order": "{{order.missing}}" }),
        ] {
            assert!(
                bind_input(&bad, &BTreeMap::new(), None, Some(&drop)).is_err(),
                "{bad}"
            );
        }
        // A drop always has its order: one without is refused, never left out.
        let no_order = Drop {
            order: None,
            ..drop.clone()
        };
        let err = bind_input(
            &serde_json::json!({ "order": "{{order.uri}}" }),
            &BTreeMap::new(),
            None,
            Some(&no_order),
        )
        .unwrap_err();
        assert!(format!("{err:?}").contains("has no value here"), "{err:?}");
    }

    /// A row action with `when` is offered only on rows whose column is
    /// truthy, and refused on any other.
    #[tokio::test]
    async fn a_row_action_with_when_is_offered_only_on_truthy_rows() {
        let (fx, root) = fixture().await;
        let row = |open: SqlCell| {
            BTreeMap::from([
                (
                    "uri".to_string(),
                    SqlCell::Text(oxplow_tasks::work_item_ref(fx.task)),
                ),
                ("open".to_string(), open),
            ])
        };
        let run = |row: BTreeMap<String, SqlCell>| {
            let (svc, root) = (fx.svc.clone(), root.clone());
            async move {
                run_lens_action(
                    &svc,
                    &root,
                    call(
                        "finish-open",
                        BTreeMap::new(),
                        Some(row),
                        Actor::Human,
                        false,
                    ),
                    &extensions::ViewContext::default(),
                )
                .await
            }
        };
        for closed in [
            SqlCell::Int(0),
            SqlCell::Null(()),
            SqlCell::Text(String::new()),
        ] {
            let err = run(row(closed)).await.unwrap_err();
            assert!(
                format!("{err:?}").contains("isn't offered on this row"),
                "{err:?}"
            );
        }
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        run(row(SqlCell::Int(1))).await.unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A lens acts as its presser and no more: one an agent pressed may run
    /// what the agent may, which is every command, and the audit names the
    /// lens and the agent's thread that pressed it.
    #[tokio::test]
    async fn a_lens_grants_no_power() {
        let (fx, root) = fixture().await;
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = run_lens_action(
            &fx.svc,
            &root,
            call("prompt", BTreeMap::new(), None, agent, false),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        let audit = fx
            .svc
            .commands
            .audit_store()
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit.actor_kind,
            oxplow_domain::events::schema::ActorKind::Lens
        );
        assert_eq!(audit.actor_id.as_deref(), Some("acme/tasks"));
        assert_eq!(audit.thread_id, Some(fx.thread));
    }

    /// A form action opens the command's form starting from its bound
    /// input, with its fixed fields; what the person fills in runs, the
    /// fixed fields put back whatever was sent. A plain action takes no
    /// input.
    #[tokio::test]
    async fn a_form_action_runs_what_the_person_filled_in_keeping_its_fixed_fields() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        let row = BTreeMap::from([
            ("uri".to_string(), SqlCell::Text(item.clone())),
            ("title".to_string(), SqlCell::Text("Old title".into())),
        ]);
        let start = action_form(
            &fx.svc,
            &root,
            call(
                "retitle",
                BTreeMap::new(),
                Some(row.clone()),
                Actor::Human,
                false,
            ),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.command.id, "oxplow.work_item.update");
        assert_eq!(start.values, json!({ "uri": item, "title": "Old title" }));
        assert_eq!(start.fixed, vec!["uri".to_string()]);

        let mut press = call("retitle", BTreeMap::new(), Some(row), Actor::Human, false);
        press.input = Some(
            json!({ "uri": "oxplow:oxplow/work_item?id=tsk999999&provider=oxplow", "title": "New title" }),
        );
        run_lens_action(&fx.svc, &root, press, &extensions::ViewContext::default())
            .await
            .unwrap();
        let title: String = fx
            .svc
            .db
            .read({
                let item = item.clone();
                move |c| {
                    c.query_row(
                        "SELECT title FROM v_work_item WHERE uri = ?1",
                        [item],
                        |r| r.get(0),
                    )
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                }
            })
            .await
            .unwrap();
        assert_eq!(
            title, "New title",
            "the fixed ref was kept, so this item changed"
        );

        let mut plain = call(
            "finish",
            BTreeMap::from([("item".to_string(), SqlCell::Text(item))]),
            None,
            Actor::Human,
            false,
        );
        plain.input = Some(json!({ "to": "canceled" }));
        let err = run_lens_action(&fx.svc, &root, plain, &extensions::ViewContext::default())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("only a form action"), "{err}");
        let err = action_form(
            &fx.svc,
            &root,
            call("finish", BTreeMap::new(), None, Actor::Human, false),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("no form action"), "{err}");
    }

    /// The foundation's hand-off form starts from its thread, fixed, and
    /// picks the recipient from the other open threads and the item from
    /// its thread's open items — each field's choices from its lens.
    #[tokio::test]
    async fn the_hand_off_form_picks_from_its_choices() {
        let (fx, root) = fixture().await;
        let other =
            crate::test_fixtures::new_thread(&fx.svc, oxplow_domain::StreamId::new(1), "Docs")
                .await;
        let closed =
            crate::test_fixtures::new_thread(&fx.svc, oxplow_domain::StreamId::new(1), "Old").await;
        fx.svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.thread.close",
                json!({ "thread": closed.id.to_string() }),
                false,
            )
            .await
            .unwrap();
        let me = oxplow_domain::uri::build::thread_uri(fx.thread);
        let start = form_start(
            &fx.svc,
            &root,
            "oxplow-foundation/hand-off",
            BTreeMap::from([("uri".to_string(), SqlCell::Text(me.clone()))]),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.command.id, "oxplow.handoff.send");
        assert_eq!(start.values["from_thread"], json!(fx.thread.to_string()));
        assert_eq!(start.fixed, vec!["from_thread".to_string()]);
        let recipients: Vec<(&str, &str)> = start.choices["to_thread"]
            .iter()
            .map(|c| (c.value.as_str(), c.label.as_str()))
            .collect();
        let other_ref = other.id.to_string();
        assert_eq!(recipients, vec![(other_ref.as_str(), "Docs")]);
        assert!(start.choices["to_thread"][0].group.is_some());
        let items: Vec<&str> = start.choices["work_item"]
            .iter()
            .map(|c| c.value.as_str())
            .collect();
        assert_eq!(items, vec![work_item_ref(fx.task)]);
    }

    /// New Item is one form over `oxplow.work_item.create`: its `native`
    /// field's children are the active list's own fields (read from a lens,
    /// the read-only ones marked), its state is picked from the two a new
    /// item may start in, and its second submit keeps what a series of
    /// similar items shares.
    #[tokio::test]
    async fn the_new_item_form_asks_for_the_lists_own_fields() {
        let (fx, root) = fixture().await;
        let start = form_start(
            &fx.svc,
            &root,
            "oxplow-foundation/new-task",
            BTreeMap::new(),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.command.id, "oxplow.work_item.create");
        assert_eq!(start.values["state"], json!("todo"));
        let priority = &start.subfields["native"]
            .iter()
            .find(|f| f.name == "priority")
            .unwrap_or_else(|| panic!("{:?}", start.subfields));
        assert_eq!(priority.kind, "enum");
        assert_eq!(priority.values, ["urgent", "high", "medium", "low"]);
        assert!(!priority.read_only);
        let states: Vec<&str> = start.choices["state"]
            .iter()
            .map(|c| c.value.as_str())
            .collect();
        assert_eq!(states, ["todo", "blocked"]);
        // The list's fields are read from the capability rows, so a switch
        // reads the form again.
        assert!(
            start
                .reads
                .models
                .iter()
                .any(|m| m == "v_capability_provider"),
            "{:?}",
            start.reads
        );
    }

    /// New Stream is one form over `oxplow.stream.create`: it starts on an
    /// existing branch with a default name, and each source shows only its
    /// own fields.
    #[tokio::test]
    async fn the_new_stream_form_shows_each_sources_fields() {
        let (fx, root) = fixture().await;
        let start = form_start(
            &fx.svc,
            &root,
            "oxplow-foundation/new-stream",
            BTreeMap::new(),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.command.id, "oxplow.stream.create");
        assert_eq!(start.values["source"], json!("existing"));
        assert!(
            start.values["title"]
                .as_str()
                .unwrap()
                .starts_with("Stream "),
            "{:?}",
            start.values
        );
        let shown: Vec<(&str, bool)> = start
            .fields
            .iter()
            .flatten()
            .map(|f| (f.name(), f.visible_if().contains_key("source")))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("title", false),
                ("source", false),
                ("existing_branch", true),
                ("new_branch", true),
                ("start_point", true),
                ("worktree", true),
            ]
        );
        assert!(start.choices.contains_key("existing_branch"));
        assert!(start.choices.contains_key("worktree"));
    }

    /// A secret field is one its command keeps out of its record: a form
    /// whose command would record it is refused, and a secret never
    /// starts with a value.
    #[tokio::test]
    async fn a_secret_field_is_one_its_command_doesnt_record() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        std::fs::write(
            root.join(".oxplow/extensions/acme/lenses/leaky.yaml"),
            "title: Leaky\nviz: form\nparams: [{ name: uri }]\nquery: SELECT title FROM v_work_item WHERE uri = :uri\nform: { command: oxplow.work_item.update, defaults: { uri: '{{param.uri}}' }, fixed: [uri], fields: [{ name: title, secret: true }] }\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".oxplow/extensions/acme/lenses/kept.yaml"),
            "title: Kept\nviz: form\nquery: SELECT 'notes.txt' AS path, 'what was there' AS content\nform: { command: oxplow.file.create, fields: [path, { name: content, secret: true }] }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let params = BTreeMap::from([("uri".to_string(), SqlCell::Text(item))]);
        let ctx = extensions::ViewContext::default();
        let refused = form_start(&fx.svc, &root, "acme/leaky", params, &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("`title` is secret but `oxplow.work_item.update` records it"),
            "{refused}"
        );
        let start = form_start(&fx.svc, &root, "acme/kept", BTreeMap::new(), &ctx)
            .await
            .unwrap();
        assert_eq!(start.values["path"], json!("notes.txt"));
        assert!(start.values.get("content").is_none(), "{:?}", start.values);
    }

    /// A form lens's fixed fields keep their `defaults` on submit, and its
    /// `fields` say which to show.
    #[tokio::test]
    async fn a_form_lens_keeps_its_fixed_fields() {
        let (fx, root) = fixture().await;
        let item = work_item_ref(fx.task);
        std::fs::write(
            root.join(".oxplow/extensions/acme/lenses/edit.yaml"),
            "title: Edit\nviz: form\nparams: [{ name: uri }]\nquery: SELECT title FROM v_work_item WHERE uri = :uri\nform: { command: oxplow.work_item.update, defaults: { uri: '{{param.uri}}' }, fixed: [uri], fields: [title] }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let params = BTreeMap::from([("uri".to_string(), SqlCell::Text(item.clone()))]);
        let start = form_start(
            &fx.svc,
            &root,
            "acme/edit",
            params.clone(),
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        assert_eq!(start.fixed, vec!["uri".to_string()]);
        assert_eq!(
            start.fields,
            Some(vec![crate::extensions::FormField::Name("title".into())])
        );
        assert_eq!(start.values["uri"], json!(item));
        submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/edit".into(),
                input: json!({ "uri": "oxplow:oxplow/work_item?id=tsk999999&provider=oxplow", "title": "Edited" }),
                params,
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        let title: String = fx
            .svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT title FROM v_work_item WHERE uri = ?1",
                    [item],
                    |r| r.get(0),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(title, "Edited");
    }

    #[test]
    fn placeholders_bind_typed_or_spliced() {
        let params = BTreeMap::from([("n".to_string(), SqlCell::Int(3))]);
        let row = BTreeMap::from([("id".to_string(), SqlCell::Int(42))]);
        let bound = bind_input(
            &json!({ "a": "{{param.n}}", "b": "tsk{{row.id}}", "c": ["{{ row.id }}"], "d": 1 }),
            &params,
            Some(&row),
            None,
        )
        .unwrap();
        assert_eq!(bound, json!({ "a": 3, "b": "tsk42", "c": [42], "d": 1 }));
        let err = bind_input(&json!({ "a": "{{row.id}}" }), &params, None, None).unwrap_err();
        assert!(err.to_string().contains("row.id"), "{err}");
    }

    /// `{{row.x|json}}` is the JSON a text cell holds, typed: how a row
    /// hands a command a list it built (the Agents page's new order).
    #[test]
    fn a_json_filter_binds_a_cells_json() {
        let row = BTreeMap::from([(
            "order".to_string(),
            SqlCell::Text("[\"claude\",\"codex\"]".into()),
        )]);
        let bound = bind_input(
            &json!({ "agents": "{{row.order|json}}" }),
            &BTreeMap::new(),
            Some(&row),
            None,
        )
        .unwrap();
        assert_eq!(bound, json!({ "agents": ["claude", "codex"] }));
        let bad = BTreeMap::from([("order".to_string(), SqlCell::Text("not json".into()))]);
        let err = bind_input(
            &json!({ "agents": "{{row.order|json}}" }),
            &BTreeMap::new(),
            Some(&bad),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("isn't JSON"), "{err}");
    }

    /// `{{param.x.id}}` is the key of the URI param `x` — a fixed field's
    /// bare id from a page's `ref`.
    #[test]
    fn a_params_id_is_the_key_of_its_uri() {
        let params = BTreeMap::from([(
            "uri".to_string(),
            SqlCell::Text("oxplow:oxplow/thread?id=thr3".into()),
        )]);
        let bound = bind_input(
            &json!({ "from_thread": "{{param.uri.id}}", "t": "on {{param.uri.id}}" }),
            &params,
            None,
            None,
        )
        .unwrap();
        assert_eq!(bound, json!({ "from_thread": "thr3", "t": "on thr3" }));
    }

    /// P6.B2: a form lens submits its command as the lens, its defaults
    /// under what the person filled in.
    #[tokio::test]
    async fn a_form_submits_its_command_as_the_lens() {
        let (fx, root) = fixture().await;
        std::fs::write(
            root.join(".oxplow/extensions/acme/lenses/new-task.yaml"),
            "title: New Task\nviz: form\nparams: [{ name: body, default: from the form }]\nform: { command: oxplow.work_item.create, defaults: { body: '{{param.body}}' } }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        let out = submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/new-task".into(),
                input: json!({ "title": "Made by a form" }),
                params: BTreeMap::new(),
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap();
        let item = out.result["uri"].as_str().unwrap().to_string();
        let (kind, _) = audit_actor(&fx.svc).await;
        assert_eq!(kind, "lens");
        let row: (String, String) = fx
            .svc
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT title, body FROM v_work_item WHERE uri = ?1",
                    [item],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(row, ("Made by a form".into(), "from the form".into()));
        // A lens that isn't a form can't be submitted.
        let err = submit_form(
            &fx.svc,
            &root,
            FormSubmission {
                lens_id: "acme/tasks".into(),
                input: json!({}),
                params: BTreeMap::new(),
                on_behalf_of: Actor::Human,
                confirmed: false,
            },
            &extensions::ViewContext::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("isn't a form"), "{err}");
    }

    /// P6b.D2: a private extension with a component (`board`) that may
    /// query `acme/tasks` and invoke `oxplow.work_item.transition`.
    async fn component_fixture() -> (crate::test_fixtures::TaskEffortFixture, std::path::PathBuf) {
        let (fx, root) = fixture().await;
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { commands: [oxplow.work_item.transition] }\ncustom_components:\n  - { id: board, assets: [tasks], commands: [oxplow.work_item.transition] }\n",
        )
        .unwrap();
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        (fx, root)
    }

    /// Load `acme/board`'s bundle as its frame does: the version it runs.
    async fn load_board(fx: &crate::test_fixtures::EffortFixture, root: &Path) -> String {
        // Whatever was edited since is published first, as the owner of
        // generations does on the file change.
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        load_component(&fx.svc, root, "acme/board").unwrap()
    }

    /// An invoke of `command` from `acme/board`'s frame loaded at `version`.
    fn board_invoke(
        fx: &crate::test_fixtures::TaskEffortFixture,
        command: &str,
        version: &str,
    ) -> ComponentInvoke {
        ComponentInvoke {
            component: "acme/board".into(),
            command: command.into(),
            input: serde_json::json!({ "uri": oxplow_tasks::work_item_ref(fx.task), "to": "done" }),
            on_behalf_of: Actor::Human,
            confirmed: false,
            version: version.into(),
        }
    }

    /// `err` is a refusal saying `why`.
    fn refused(err: CommandError, why: &str) {
        assert!(
            matches!(&err, CommandError::Denied { reason }
                if reason.contains("component `acme/board`") && reason.contains(why)),
            "{err:?}"
        );
    }

    /// tsk984: what acts is what the frame loaded. A frame keeps acting at
    /// the version it loaded whatever the disk says later. A version that
    /// isn't loaded, or is another component's, is refused.
    #[tokio::test]
    async fn an_invoke_acts_only_for_a_version_that_is_loaded() {
        let (fx, root) = component_fixture().await;
        let index = root.join(".oxplow/extensions/acme/components/board/index.html");
        let loaded = load_board(&fx, &root).await;
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", "0123abcd"),
            )
            .await
            .unwrap_err(),
            "isn't loaded",
        );
        std::fs::write(&index, "<!doctype html><p>later</p>").unwrap();
        invoke_component_command(
            &fx.svc,
            &root,
            board_invoke(&fx, "oxplow.work_item.transition", &loaded),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A hot update: once a changed bundle is activated, a frame still on
    /// the old version can't act — it's told to reload — and the version
    /// it reloads at acts.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_activated_edit_needs_a_reload() {
        let (fx, root) = component_fixture().await;
        let activate = || async {
            crate::test_fixtures::activate_with_problems(&fx.svc).await;
        };
        activate().await;
        let old = load_board(&fx, &root).await;

        std::fs::write(
            root.join(".oxplow/extensions/acme/components/board/index.html"),
            "<!doctype html><p>2</p>",
        )
        .unwrap();
        activate().await;
        refused(
            invoke_component_command(
                &fx.svc,
                &root,
                board_invoke(&fx, "oxplow.work_item.transition", &old),
            )
            .await
            .unwrap_err(),
            "reload its lens",
        );
        let new = load_board(&fx, &root).await;
        assert_ne!(new, old);
        assert_ne!(state(&fx.svc, fx.task).await, "done");
        invoke_component_command(
            &fx.svc,
            &root,
            board_invoke(&fx, "oxplow.work_item.transition", &new),
        )
        .await
        .unwrap();
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A frame is served from its deployment on disk: an open frame on an
    /// older version keeps loading that version's files while its
    /// deployment is in the store, and once the deployment is pruned the
    /// frame has nothing to load and must reload its lens.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_open_frame_is_served_from_its_deployment_on_disk() {
        let (fx, root) = component_fixture().await;
        let first = load_board(&fx, &root).await;
        let first_hash = fx.svc.extension_catalog.live()["acme"].clone();
        std::fs::write(
            root.join(".oxplow/extensions/acme/components/board/index.html"),
            "<!doctype html><p>2</p>",
        )
        .unwrap();
        let second = load_board(&fx, &root).await;
        assert_ne!(first, second);
        let file = |version: &str| {
            fx.svc
                .component_bundles
                .get(version)
                .and_then(|bundle| bundle.file(""))
        };
        assert_eq!(file(&first).as_deref(), Some(&b"<!doctype html>"[..]));
        assert_eq!(
            file(&second).as_deref(),
            Some(&b"<!doctype html><p>2</p>"[..])
        );

        crate::extension_store::remove(&root, "acme", &[first_hash]);
        assert_eq!(file(&first), None);
        assert!(file(&second).is_some());
    }

    /// tsk984: the bundle is read as it is served — the extension's folder
    /// joined with the manifest's `bundle` — so on a case-insensitive disk a
    /// spelling that differs only in case still covers every file it
    /// serves, and an edit is a new version.
    #[tokio::test]
    async fn a_bundle_spelled_in_another_case_is_hashed_as_served() {
        let (fx, root) = fixture().await;
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        if !ext.join("components/BOARD").exists() {
            // A case-sensitive disk: the manifest's spelling is a missing
            // folder there, a load error, never a silent hash.
            return;
        }
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { commands: [oxplow.work_item.transition] }\ncustom_components:\n  - { id: board, bundle: components/Board, assets: [tasks], commands: [oxplow.work_item.transition] }\n",
        )
        .unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        let first = load_board(&fx, &root).await;
        let served = fx.svc.component_bundles.get(&first).unwrap();
        assert_eq!(served.file("").as_deref(), Some(&b"<!doctype html>"[..]));
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>2").unwrap();
        assert_ne!(load_board(&fx, &root).await, first);
    }

    /// P11 (tsk960): a component that declares no commands can only show
    /// and query.
    #[tokio::test]
    async fn a_component_without_commands_can_query() {
        let (fx, root) = fixture().await;
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\ncustom_components:\n  - { id: board, assets: [tasks] }\n",
        )
        .unwrap();
        std::fs::create_dir_all(ext.join("components/board")).unwrap();
        std::fs::write(ext.join("components/board/index.html"), "<!doctype html>").unwrap();
        std::fs::write(
            ext.join("lenses/view.yaml"),
            "title: View\nquery: SELECT 1 AS n\nviz: custom\ncustom: { component: board }\n",
        )
        .unwrap();
        let ctx = extensions::ViewContext::default();
        run_component_query(
            &fx.svc,
            &root,
            "acme/board",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_component_queries_only_its_declared_lenses() {
        let (fx, root) = component_fixture().await;
        let ctx = extensions::ViewContext::default();
        let run = run_component_query(
            &fx.svc,
            &root,
            "acme/board",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap();
        assert_eq!(run.lens.id, "acme/tasks");
        let err = run_component_query(
            &fx.svc,
            &root,
            "acme/board",
            "acme/view",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("isn't one of component `board`'s assets (acme/tasks)")),
            "{err:?}"
        );
        let err = run_component_query(
            &fx.svc,
            &root,
            "acme/nope",
            "tasks",
            BTreeMap::new(),
            &ctx,
            &load_board(&fx, &root).await,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("`acme` has no custom component `nope`"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_component_invokes_only_its_declared_commands_as_itself() {
        let (fx, root) = component_fixture().await;
        let version = load_board(&fx, &root).await;
        let invoke = |command: &str| board_invoke(&fx, command, &version);
        let err = invoke_component_command(&fx.svc, &root, invoke("oxplow.config.set"))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("isn't one of component `board`'s commands"),
            "{err}"
        );
        let out = invoke_component_command(&fx.svc, &root, invoke("oxplow.work_item.transition"))
            .await
            .unwrap();
        let audit = fx
            .svc
            .commands
            .audit_store()
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            audit.actor_kind,
            oxplow_domain::events::schema::ActorKind::Component
        );
        assert_eq!(audit.actor_id.as_deref(), Some("acme/board"));
        assert_eq!(state(&fx.svc, fx.task).await, "done");
    }

    /// A component that declares `sql: read` runs its own read-only SQL
    /// over the models, held to its extension's `permissions.sql`; one that
    /// doesn't declare it is refused.
    #[tokio::test]
    async fn a_component_with_sql_read_queries_the_layer() {
        let (fx, root) = component_fixture().await;
        let ext = root.join(".oxplow/extensions/acme");
        let ask = |sql: &str, version: String| {
            run_component_sql(
                &fx.svc,
                &root,
                ComponentSql {
                    component: "acme/board".into(),
                    sql: sql.into(),
                    params: vec![SqlCell::Int(2)],
                    stream: None,
                    limit: None,
                    on_behalf_of: Actor::Human,
                    version,
                },
            )
        };
        let err = ask("SELECT ?1 AS n", load_board(&fx, &root).await)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("component `board` doesn't declare `sql: read`"),
            "{err}"
        );
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { sql: read, commands: [oxplow.work_item.transition] }\ncustom_components:\n  - { id: board, assets: [tasks], commands: [oxplow.work_item.transition], sql: read }\n",
        )
        .unwrap();
        let version = load_board(&fx, &root).await;
        let out = ask("SELECT ?1 AS n", version.clone()).await.unwrap();
        assert_eq!(out.rows, vec![vec![SqlCell::Int(2)]]);
        // Read-only, as any query over the models is.
        assert!(ask("DELETE FROM v_work_item", version).await.is_err());
    }

    /// The Timeline's own queries, through the component SQL path.
    #[tokio::test]
    async fn the_timeline_queries_read_through_component_sql() {
        let (fx, root) = component_fixture().await;
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { sql: read, commands: [oxplow.work_item.transition] }\ncustom_components:\n  - { id: board, assets: [tasks], commands: [oxplow.work_item.transition], sql: read }\n",
        )
        .unwrap();
        let version = load_board(&fx, &root).await;
        let window = vec![
            SqlCell::Text("2000-01-01T00:00:00.000Z".into()),
            SqlCell::Text("2100-01-01T00:00:00.000Z".into()),
            SqlCell::Null(()),
        ];
        for sql in [
            "SELECT 'effort', uri, COALESCE(NULLIF(title, ''), uri), started_at, ended_at FROM v_effort WHERE started_at < ?2 AND COALESCE(ended_at, ?2) > ?1 AND (?3 IS NULL OR thread_id = ?3) ORDER BY started_at",
            "SELECT 'commit', uri, subject, committed_at, NULL FROM v_commit WHERE committed_at >= ?1 AND committed_at < ?2 AND ?3 IS NULL ORDER BY committed_at",
            "SELECT 'thread', uri, title, created_at, closed_at FROM v_thread WHERE created_at < ?2 AND COALESCE(closed_at, ?2) > ?1 AND (?3 IS NULL OR uri = 'oxplow:oxplow/thread?id=thr' || ?3) ORDER BY created_at",
        ] {
            let direct = fx.svc.sql.query_sql(sql, window.clone(), None).await.unwrap();
            let out = run_component_sql(
                &fx.svc,
                &root,
                ComponentSql {
                    component: "acme/board".into(),
                    sql: sql.into(),
                    params: window.clone(),
                    stream: None,
                    limit: None,
                    on_behalf_of: Actor::Human,
                    version: version.clone(),
                },
            )
            .await
            .unwrap();
            assert_eq!(out.rows.len(), direct.rows.len(), "{sql}");
            eprintln!("{} rows: {}", &sql[8..16], out.rows.len());
        }
    }

    /// A component opens the form of one of its own commands in the
    /// host, starting from the values it gives; any other command is
    /// refused.
    #[tokio::test]
    async fn a_component_opens_the_form_of_its_own_commands_only() {
        let (fx, root) = component_fixture().await;
        let version = load_board(&fx, &root).await;
        let start = component_form(
            &fx.svc,
            &root,
            "acme/board",
            "oxplow.work_item.transition",
            serde_json::json!({ "to": "done" }),
            &version,
        )
        .unwrap();
        assert_eq!(start.command.id, "oxplow.work_item.transition");
        assert_eq!(start.values, serde_json::json!({ "to": "done" }));
        let err = component_form(
            &fx.svc,
            &root,
            "acme/board",
            "oxplow.config.set",
            serde_json::json!({}),
            &version,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("isn't one of component `board`'s commands"),
            "{err}"
        );
    }

    /// The frame's input is checked like anyone's: an array or a scalar
    /// where the command takes an object is `Invalid`, and nothing runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_component_invoke_with_a_non_object_input_is_invalid() {
        let (fx, root) = component_fixture().await;
        let version = load_board(&fx, &root).await;
        for input in [
            serde_json::json!([1, 2]),
            serde_json::json!("done"),
            serde_json::json!(7),
        ] {
            let err = invoke_component_command(
                &fx.svc,
                &root,
                ComponentInvoke {
                    component: "acme/board".into(),
                    command: "oxplow.work_item.transition".into(),
                    input: input.clone(),
                    on_behalf_of: Actor::Human,
                    confirmed: false,
                    version: version.clone(),
                },
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{input}: {err:?}"
            );
        }
        assert_ne!(state(&fx.svc, fx.task).await, "done");
    }
}
