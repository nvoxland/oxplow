//! What an action point offers where a page places it
//! (`.context/extensions-slots.md` "Action points and the store filter"):
//! the commands that joined it (`ui.at`), each with the point's context as
//! its input, and how the shell draws them — nothing with no contributor,
//! a button with one, a menu with several. The window asks
//! [`action_point_offer`] once per placed point and draws what it says; the
//! decision is made here and nowhere else.

use oxplow_domain::{Actor, CommandSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::ref_commands::{offered, OfferKey};
use crate::extension::extensions::action_points::ActionPointDecl;
use crate::Services;

/// How the shell draws a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Draws {
    /// No contributor: nothing is drawn.
    None,
    /// One: a button that runs it.
    Button,
    /// Several: a menu of them.
    Menu,
}

impl Draws {
    /// The drawing rule: by how many commands joined the point.
    pub fn for_count(n: usize) -> Draws {
        match n {
            0 => Draws::None,
            1 => Draws::Button,
            _ => Draws::Menu,
        }
    }
}

/// One command at a point, and what it runs with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct PointCommand {
    pub id: String,
    pub label: String,
    /// What it runs with: its `ui.input` bound from the point's context.
    #[specta(type = oxplow_domain::Json)]
    pub input: Value,
}

/// What a point offers for a context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct PointOffer {
    pub point: String,
    /// The point's label, which names its button or menu when a command's
    /// own label doesn't.
    pub label: String,
    pub draws: Draws,
    /// By extension, then command id.
    pub commands: Vec<PointCommand>,
}

/// `context` checked against the point's declaration, as the input its
/// commands get: its declared keys with a value (an optional key left out
/// or null is dropped). `Ok(None)`: a required key has no value here, so
/// the point is not offered; an undeclared key or a value of the wrong
/// type is an error.
pub fn point_input(decl: &ActionPointDecl, context: &Value) -> Result<Option<Value>, String> {
    let empty = Map::new();
    let given = match context {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(format!("`{}`: the context is an object", decl.id)),
    };
    if let Some(key) = given
        .keys()
        .find(|k| !decl.context.iter().any(|d| &d.name == *k))
    {
        return Err(format!(
            "`{}` takes no context key `{key}` (it takes {})",
            decl.id,
            decl.context
                .iter()
                .map(|k| format!("`{}`", k.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut input = Map::new();
    for key in &decl.context {
        match given.get(&key.name).filter(|v| !v.is_null()) {
            None if key.optional => {}
            None => return Ok(None),
            Some(v) if key.ty.holds(v) => {
                input.insert(key.name.clone(), v.clone());
            }
            Some(v) => {
                return Err(format!(
                    "`{}`: context `{}` is {:?}, not {v}",
                    decl.id, key.name, key.ty
                ))
            }
        }
    }
    Ok(Some(Value::Object(input)))
}

/// [`action_point_offer`] over `specs` (the commands a person may run).
pub fn point_offer(
    decl: &ActionPointDecl,
    specs: &[CommandSpec],
    context: &Value,
) -> Result<PointOffer, String> {
    let nothing = || PointOffer {
        point: decl.id.clone(),
        label: decl.label.clone(),
        draws: Draws::None,
        commands: Vec::new(),
    };
    let Some(Value::Object(context)) = point_input(decl, context)? else {
        return Ok(nothing());
    };
    let key = OfferKey::Point(decl.id.clone());
    let mut commands: Vec<PointCommand> = offered(specs, &key, &Default::default(), "store")
        .into_iter()
        .map(|s| PointCommand {
            id: s.id.clone(),
            label: s
                .ui
                .as_ref()
                .map(|ui| ui.label.clone())
                .unwrap_or_else(|| s.id.clone()),
            input: s
                .ui
                .as_ref()
                .map(|ui| ui.point_input(&context))
                .unwrap_or_else(|| Value::Object(context.clone())),
        })
        .collect();
    // By extension, then command id (a command's id starts with its
    // extension's namespace).
    commands.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(PointOffer {
        draws: Draws::for_count(commands.len()),
        commands,
        ..nothing()
    })
}

/// What action point `point` offers for `context`, among the enabled
/// extensions' commands. An unknown point is an error (a page placing a
/// point nobody declared is a typo, not "nothing to draw").
pub async fn action_point_offer(
    svc: &Services,
    point: &str,
    context: &Value,
) -> Result<PointOffer, String> {
    let decl = svc
        .extension_catalog
        .unpinned(
            &svc.layout.project_dir,
            "an action point's offer reads one snapshot",
        )
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.action_points.iter())
        .find(|p| p.id == point)
        .cloned()
        .ok_or_else(|| format!("`{point}` is no action point an enabled extension declares"))?;
    point_offer(&decl, &svc.commands.list(&Actor::Human), context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::extensions::action_points::parse_action_points;
    use oxplow_domain::{Access, Atomicity, Confirm, Lifecycle};
    use serde_json::json;

    fn point() -> ActionPointDecl {
        let yaml =
            "- { name: task.new, label: New Task, context: { parent?: uri, thread_id: int } }";
        let value = serde_yaml::from_str(yaml).unwrap();
        parse_action_points(&value, "extension.yaml", "", "oxplow")
            .0
            .remove(0)
    }

    fn at(id: &str, label: &str, point: &str) -> CommandSpec {
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
            ui: Some(serde_json::from_value(json!({ "label": label, "at": point })).unwrap()),
            op: None,
            unrecorded: vec![],
        }
    }

    /// Nothing with no contributor, a button with one, a menu with several;
    /// a command at another point does not count.
    #[test]
    fn a_point_draws_by_its_contributors() {
        let ctx = json!({ "thread_id": 3 });
        let mut specs = vec![at("x.other", "Other", "oxplow/other")];
        let offer = |specs: &[CommandSpec]| point_offer(&point(), specs, &ctx).unwrap();
        assert_eq!(offer(&specs).draws, Draws::None);
        specs.push(at("a.new", "Zed", "oxplow/task.new"));
        let one = offer(&specs);
        assert_eq!((one.draws, one.commands.len()), (Draws::Button, 1));
        specs.push(at("b.new", "Alpha", "oxplow/task.new"));
        let many = offer(&specs);
        assert_eq!(many.draws, Draws::Menu);
        let ids: Vec<_> = many.commands.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a.new", "b.new"]);
    }

    /// Each command gets the point's context as its input: the declared
    /// keys with a value, an optional one left out when null.
    #[test]
    fn the_context_is_the_commands_input() {
        let specs = vec![at("a.new", "New", "oxplow/task.new")];
        let full = json!({ "parent": "oxplow:oxplow/task?id=1", "thread_id": 3 });
        let offer = point_offer(&point(), &specs, &full).unwrap();
        assert_eq!(offer.commands[0].input, full);
        let part = json!({ "parent": null, "thread_id": 3 });
        let offer = point_offer(&point(), &specs, &part).unwrap();
        assert_eq!(offer.commands[0].input, json!({ "thread_id": 3 }));
    }

    /// A command's `ui.input` binds the context's keys by name; a key the
    /// page left out stays unset.
    #[test]
    fn a_commands_input_binds_the_context_keys() {
        let mut spec = at("a.new", "New", "oxplow/task.new");
        spec.ui = Some(
            serde_json::from_value(json!({ "label": "New", "at": "oxplow/task.new",
                "input": { "under": "{{context.parent}}", "thread": "{{context.thread_id}}" } }))
            .unwrap(),
        );
        let offer = point_offer(&point(), &[spec], &json!({ "thread_id": 3 })).unwrap();
        assert_eq!(offer.commands[0].input, json!({ "thread": 3 }));
    }

    /// A required key without a value offers nothing; an undeclared key or
    /// a value of the wrong type is an error.
    #[test]
    fn a_context_is_checked_against_the_declaration() {
        let specs = vec![at("a.new", "New", "oxplow/task.new")];
        let none = point_offer(&point(), &specs, &json!({})).unwrap();
        assert_eq!(none.draws, Draws::None);
        let err = point_offer(&point(), &specs, &json!({ "thread_id": 1, "x": 1 })).unwrap_err();
        assert!(err.contains("no context key `x`"), "{err}");
        let err = point_offer(&point(), &specs, &json!({ "thread_id": "one" })).unwrap_err();
        assert!(err.contains("thread_id"), "{err}");
    }
}
