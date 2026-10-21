//! How a ref of the old grammar (`<kind>:<id>[@rev][#frag]`) reads as an
//! oxplow URI (`oxplow:<extension>/<resource>?<params>#<view>`): the one
//! place that knowledge survives, for the migration that rewrote every
//! stored ref (`.context/resources.md`). Frozen: nothing builds an old ref
//! any more, so this table never grows.

use std::collections::BTreeMap;

mod v1;

use oxplow_domain::uri::oxplow_uri::{OxplowUri, ResourceName};
use v1::Uri as OldRef;

/// The kind of the old ref `text`, or `None` when it isn't one.
pub fn v1_kind_of(text: &str) -> Option<String> {
    if text.starts_with("oxplow:") {
        return None;
    }
    OldRef::parse(text).ok().map(|r| r.kind)
}

/// An old symbol id's path and symbol: `<path>/<symbol>` as the builder
/// spelled it, or `<path>::<name>` as the metric scans did — told apart by
/// what comes before the first `::`, a file name (`uses.rs`) for the
/// scans' form.
fn symbol_parts(id: &str) -> Option<(&str, &str)> {
    if let Some((path, name)) = id.split_once("::") {
        let file = path.rsplit('/').next().unwrap_or(path);
        if file.contains('.') && !name.is_empty() {
            return Some((path, name));
        }
    }
    id.rsplit_once('/')
}

/// The URI `text` names, or `None` when `text` isn't an old ref this can
/// read (left as it was by the caller). `extension` is the extension that
/// declared `text`'s kind, for a kind an extension declared
/// (`ref_kind.extension`), else `None`.
pub fn v1_to_uri(text: &str, extension: Option<&str>) -> Option<String> {
    if text.starts_with("oxplow:") {
        return None;
    }
    // Ids the old grammar couldn't hold as written: a raw `@`, a `#`.
    if let Some(id) = text.strip_prefix("extension_deployment:") {
        let (name, hash) = id.rsplit_once('@')?;
        return ours(
            "extension_deployment",
            &[("name", name), ("hash", hash)],
            &[],
        );
    }
    if let Some(id) = text.strip_prefix("test:") {
        let (class, name) = id.split_once("::")?;
        return ours("test", &[("class", class), ("name", name)], &[]);
    }
    let old = OldRef::parse(text).ok()?;
    let id = old.id.as_str();
    let rev: Vec<(&str, &str)> = old.rev.as_deref().map(|r| ("rev", r)).into_iter().collect();
    let lines = |frag: Option<&str>| -> Vec<(&'static str, String)> {
        match frag {
            Some(f) => match f.strip_prefix('L') {
                Some(lines) => vec![("lines", lines.to_string())],
                None => vec![("part", f.to_string())],
            },
            None => Vec::new(),
        }
    };
    let view = lines(old.frag.as_deref());
    let view: Vec<(&str, &str)> = view.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let with = |params: &[(&str, &str)]| -> Vec<(String, String)> {
        params
            .iter()
            .chain(rev.iter())
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let one = |key: &str| ours_owned(&old.kind, with(&[(key, id)]), &view);
    match old.kind.as_str() {
        "project" | "stream" | "thread" | "effort" | "turn" | "snapshot" | "comment" | "event"
        | "answer" | "finding" | "thread_note" | "agent_session" | "handoff" | "dashboard"
        | "run" | "proposal" | "claim" | "decision" | "job_run" | "ai_call" => one("id"),
        "commit" => one("sha"),
        "branch" | "ai_model" | "extension" | "consumer" => one("name"),
        "wiki" => one("slug"),
        "config" => one("key"),
        "nudge" => one("kind"),
        "file" | "dir" => one("path"),
        "symbol" => {
            let (path, symbol) = symbol_parts(id)?;
            ours_owned("symbol", with(&[("path", path), ("symbol", symbol)]), &view)
        }
        "work_item" => {
            let (provider, item) = id.split_once(':')?;
            ours_owned(
                "work_item",
                with(&[("provider", provider), ("id", item)]),
                &view,
            )
        }
        "subagent" => {
            let (session, agent) = id.split_once('/')?;
            ours_owned(
                "subagent",
                with(&[("session", session), ("agent", agent)]),
                &view,
            )
        }
        "block" => {
            let (path, lines) = id.rsplit_once(':')?;
            ours_owned("block", with(&[("path", path), ("lines", lines)]), &view)
        }
        kind if is_contribution(kind) => {
            // A lens's, page's or metric's own params rode in its id.
            let (head, params) = v1::view_of(id);
            let mut all: Vec<(String, String)> = vec![(kind.to_string(), head)];
            for (k, v) in params {
                // A param that is itself an old ref is rewritten too.
                let v = match k.as_str() {
                    "ref" | "parent" => v1_to_uri(&v, None).unwrap_or(v),
                    _ => v,
                };
                all.push((k, v));
            }
            ours_owned(kind, all, &view)
        }
        kind => {
            let extension = extension?;
            let prefix = format!(
                "{}_",
                oxplow_domain::contribution::extension_namespace(extension)
            );
            let name = kind.strip_prefix(&prefix).filter(|n| !n.is_empty())?;
            let resource = ResourceName::new(extension, name).ok()?;
            build(resource, with(&[("id", id)]), &view)
        }
    }
}

/// A kind of contribution (`lens`, `page`, `collector`, …) as the old
/// grammar named it: its id is the resource's key param, named for it.
/// (`ref_kind` is left out: no stored ref names one.)
fn is_contribution(kind: &str) -> bool {
    const KINDS: &[&str] = &[
        "model",
        "measure",
        "metric",
        "dimension",
        "collector",
        "event_type",
        "command",
        "job",
        "ai_profile",
        "effect",
        "advisory",
        "skill",
        "implementation",
        "provider",
        "lens",
        "page",
        "panel",
        "slot",
        "decorator",
        "replacement",
        "custom_component",
    ];
    KINDS.contains(&kind)
}

fn ours(name: &str, params: &[(&str, &str)], view: &[(&str, &str)]) -> Option<String> {
    ours_owned(
        name,
        params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        view,
    )
}

fn ours_owned(name: &str, params: Vec<(String, String)>, view: &[(&str, &str)]) -> Option<String> {
    build(ResourceName::new("oxplow", name).ok()?, params, view)
}

fn build(
    resource: ResourceName,
    params: Vec<(String, String)>,
    view: &[(&str, &str)],
) -> Option<String> {
    let params: BTreeMap<String, String> = params.into_iter().collect();
    let view: BTreeMap<String, String> = view
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    OxplowUri::build(resource, params, view)
        .ok()
        .map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::v1_to_uri;

    fn to(text: &str) -> Option<String> {
        v1_to_uri(text, None)
    }

    #[test]
    fn an_entity_id_is_its_id_param() {
        for (old, new) in [
            ("effort:eff12", "oxplow:oxplow/effort?id=eff12"),
            ("thread:thr3", "oxplow:oxplow/thread?id=thr3"),
            ("stream:str1", "oxplow:oxplow/stream?id=str1"),
            ("turn:trn7", "oxplow:oxplow/turn?id=trn7"),
            ("snapshot:42", "oxplow:oxplow/snapshot?id=42"),
            ("agent_session:ses3", "oxplow:oxplow/agent_session?id=ses3"),
            ("decision:3", "oxplow:oxplow/decision?id=3"),
            (
                "event:0b9e6e1c-1111-4222-8333-444455556666",
                "oxplow:oxplow/event?id=0b9e6e1c-1111-4222-8333-444455556666",
            ),
            ("project:oxplow", "oxplow:oxplow/project?id=oxplow"),
        ] {
            assert_eq!(to(old).as_deref(), Some(new), "{old}");
        }
    }

    #[test]
    fn a_shaped_id_is_its_named_parts() {
        for (old, new) in [
            ("commit:4c44d495", "oxplow:oxplow/commit?sha=4c44d495"),
            ("branch:main", "oxplow:oxplow/branch?name=main"),
            ("wiki:auth-flow", "oxplow:oxplow/wiki?slug=auth-flow"),
            (
                "work_item:oxplow:tsk42",
                "oxplow:oxplow/work_item?id=tsk42&provider=oxplow",
            ),
            (
                "work_item:issues:ENG-12",
                "oxplow:oxplow/work_item?id=ENG-12&provider=issues",
            ),
            (
                "symbol:src/lib.rs/Widget::spin",
                "oxplow:oxplow/symbol?path=src/lib.rs&symbol=Widget::spin",
            ),
            // The metric scans spelled a function `<path>::<name>`.
            (
                "symbol:crates/x/src/uses.rs::builtins_used",
                "oxplow:oxplow/symbol?path=crates/x/src/uses.rs&symbol=builtins_used",
            ),
            (
                "symbol:README.md/w.py/Widget::value~2",
                "oxplow:oxplow/symbol?path=README.md/w.py&symbol=Widget::value~2",
            ),
            (
                "subagent:ses3/agent-7f",
                "oxplow:oxplow/subagent?agent=agent-7f&session=ses3",
            ),
            (
                "config:newThreadSession",
                "oxplow:oxplow/config?key=newThreadSession",
            ),
            (
                "ai_model:claude-opus-4-8",
                "oxplow:oxplow/ai_model?name=claude-opus-4-8",
            ),
            (
                "extension:oxplow-bundled",
                "oxplow:oxplow/extension?name=oxplow-bundled",
            ),
            (
                "block:src/a.rs:10-20",
                "oxplow:oxplow/block?lines=10-20&path=src/a.rs",
            ),
            (
                "test:a.b.C::it works",
                "oxplow:oxplow/test?class=a.b.C&name=it%20works",
            ),
            ("nudge:no-tests", "oxplow:oxplow/nudge?kind=no-tests"),
            (
                "consumer:oxplow.page_refs",
                "oxplow:oxplow/consumer?name=oxplow.page_refs",
            ),
        ] {
            assert_eq!(to(old).as_deref(), Some(new), "{old}");
        }
        // Its id held a raw `@` the old grammar never allowed.
        assert_eq!(
            to("extension_deployment:acme@3f9a").as_deref(),
            Some("oxplow:oxplow/extension_deployment?hash=3f9a&name=acme")
        );
    }

    #[test]
    fn a_revision_is_a_param_and_a_fragment_the_view() {
        for (old, new) in [
            ("file:src/a.rs", "oxplow:oxplow/file?path=src/a.rs"),
            (
                "file:src/a%40b.rs@git:HEAD#L10-20",
                "oxplow:oxplow/file?path=src/a@b.rs&rev=git:HEAD#lines=10-20",
            ),
            (
                "file:src/a.rs#L3",
                "oxplow:oxplow/file?path=src/a.rs#lines=3",
            ),
            (
                "dir:crates/app@snap:7",
                "oxplow:oxplow/dir?path=crates/app&rev=snap:7",
            ),
            (
                "event:0b9e6e1c-1111-4222-8333-444455556666#output",
                "oxplow:oxplow/event?id=0b9e6e1c-1111-4222-8333-444455556666#part=output",
            ),
            (
                "ai_call:12#request",
                "oxplow:oxplow/ai_call?id=12#part=request",
            ),
        ] {
            assert_eq!(to(old).as_deref(), Some(new), "{old}");
        }
    }

    /// A param no URI can carry (a key that isn't one) leaves the old ref
    /// as it was rather than failing the migration.
    #[test]
    fn a_ref_whose_param_cant_be_a_uris_is_left_as_it_was() {
        assert_eq!(to("page:acme/x?a]b=1"), None);
    }

    #[test]
    fn a_contribution_is_its_key_param_and_a_page_or_lens_keeps_its_params() {
        for (old, new) in [
            (
                "collector:triage/todos",
                "oxplow:oxplow/collector?collector=triage/todos",
            ),
            (
                "command:oxplow.file.save",
                "oxplow:oxplow/command?command=oxplow.file.save",
            ),
            (
                "job:effort-review/review",
                "oxplow:oxplow/job?job=effort-review/review",
            ),
            (
                "metric:oxplow.todos",
                "oxplow:oxplow/metric?metric=oxplow.todos",
            ),
            (
                "metric:oxplow.todos?range=7d&branch=main",
                "oxplow:oxplow/metric?branch=main&metric=oxplow.todos&range=7d",
            ),
            (
                "lens:acme/blocked?stream_id=2&thread_id=5",
                "oxplow:oxplow/lens?lens=acme/blocked&stream_id=2&thread_id=5",
            ),
            ("page:tasks", "oxplow:oxplow/page?page=tasks"),
            (
                "page:settings?area=ai",
                "oxplow:oxplow/page?area=ai&page=settings",
            ),
            (
                "page:github/open-prs",
                "oxplow:oxplow/page?page=github/open-prs",
            ),
            (
                "panel:oxplow-bundled/work",
                "oxplow:oxplow/panel?panel=oxplow-bundled/work",
            ),
            ("model:effort", "oxplow:oxplow/model?model=effort"),
        ] {
            assert_eq!(to(old).as_deref(), Some(new), "{old}");
        }
        // A param that is itself a ref is rewritten too.
        assert_eq!(
            to("page:new-task?parent=work_item:oxplow:tsk1").as_deref(),
            Some("oxplow:oxplow/page?page=new-task&parent=oxplow:oxplow/work_item?id%3Dtsk1%26provider%3Doxplow")
        );
        assert_eq!(
            to("page:acme/pr?ref=acme_pr:12").as_deref(),
            Some("oxplow:oxplow/page?page=acme/pr&ref=acme_pr:12"),
            "an extension's kind it can't place without its extension is left as it was"
        );
    }

    #[test]
    fn an_extensions_kind_is_that_extensions_resource() {
        assert_eq!(
            v1_to_uri("acme_pr:12", Some("acme")).as_deref(),
            Some("oxplow:acme/pr?id=12")
        );
        assert_eq!(
            v1_to_uri("acme_tools_pr:12", Some("acme-tools")).as_deref(),
            Some("oxplow:acme-tools/pr?id=12")
        );
        assert_eq!(
            v1_to_uri("github_pr:7", Some("github")).as_deref(),
            Some("oxplow:github/pr?id=7")
        );
        // Without the extension it can't be placed.
        assert_eq!(to("acme_pr:12"), None);
    }

    #[test]
    fn what_isnt_an_old_ref_is_left_alone() {
        for text in [
            "",
            "oxplow:tsk42",
            "tsk42",
            "not a ref",
            "https://example.com/a",
            "oxplow:oxplow/effort?id=eff1",
        ] {
            assert_eq!(to(text), None, "{text}");
        }
    }
}
