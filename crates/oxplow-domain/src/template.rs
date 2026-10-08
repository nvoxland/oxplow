//! `{{scope.name}}` placeholders: the one tokenizer for every template an
//! extension or core writes — a lens action's input (`{{param.x}}`,
//! `{{row.x}}`), a command's `ui.input` (`{{stream}}`, `{{thread}}`,
//! `{{ref}}`, `{{ref.id}}`) and its `ui.open_after` (`{{result.x}}`).
//! What checks a template at load and what binds it at run read the same
//! spans, so they can't disagree. See `.context/commands.md` "Offering a
//! command to a person".

/// One `{{scope.name}}` placeholder in a string: its scope and name
/// (trimmed; `name` is empty without a `.`) and its byte span, from the
/// opening `{{` to just past the closing `}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    pub scope: String,
    pub name: String,
    pub start: usize,
    pub end: usize,
}

impl Placeholder {
    /// `scope`, or `scope.name`: how a template names what it binds.
    pub fn key(&self) -> String {
        if self.name.is_empty() {
            self.scope.clone()
        } else {
            format!("{}.{}", self.scope, self.name)
        }
    }
}

/// The placeholders in `s`, in order. A `{{` without a closing `}}` ends
/// the scan.
pub fn placeholders(s: &str) -> Vec<Placeholder> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = s[at..].find("{{") {
        let start = at + open;
        let Some(close) = s[start..].find("}}") else {
            break;
        };
        let end = start + close + 2;
        let inner = s[start + 2..start + close].trim();
        let (scope, name) = inner.split_once('.').unwrap_or((inner, ""));
        out.push(Placeholder {
            scope: scope.to_string(),
            name: name.to_string(),
            start,
            end,
        });
        at = end;
    }
    out
}

/// The placeholder `s` is exactly (ignoring surrounding whitespace) — a
/// value bound typed rather than spliced into text.
pub fn whole_placeholder(s: &str) -> Option<Placeholder> {
    let t = s.trim();
    match placeholders(t).as_slice() {
        [only] if only.start == 0 && only.end == t.len() => Some(only.clone()),
        _ => None,
    }
}

/// `s` with each placeholder replaced by what `value` gives it.
pub fn splice<E>(
    s: &str,
    mut value: impl FnMut(&Placeholder) -> Result<String, E>,
) -> Result<String, E> {
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for p in placeholders(s) {
        out.push_str(&s[at..p.start]);
        out.push_str(&value(&p)?);
        at = p.end;
    }
    out.push_str(&s[at..]);
    Ok(out)
}

/// Every string in `value` (a JSON template), in order.
pub fn strings(value: &serde_json::Value) -> Vec<&str> {
    fn walk<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
        match v {
            serde_json::Value::String(s) => out.push(s),
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tokenizer: spans, a string that is exactly one placeholder
    /// (bound typed), a stray `{{`, and splicing all read the same scan.
    #[test]
    fn placeholders_are_one_tokenizer() {
        let ps = placeholders("Fix {{row.title}} in {{ param.stream_id }}");
        assert_eq!(
            ps.iter()
                .map(|p| (p.scope.as_str(), p.name.as_str(), p.start, p.end))
                .collect::<Vec<_>>(),
            vec![("row", "title", 4, 17), ("param", "stream_id", 21, 42)]
        );
        assert_eq!(ps[1].key(), "param.stream_id");
        assert_eq!(
            whole_placeholder("  {{thread}} ").map(|p| p.key()),
            Some("thread".to_string())
        );
        assert!(whole_placeholder("{{row.a}} {{row.b}}").is_none());
        assert!(whole_placeholder("x {{row.a}}").is_none());
        // A stray opener is part of the next placeholder's text.
        let nested = placeholders("{{{{row.a}}");
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].scope, "{{row");
        assert_eq!(
            splice::<()>("page:x?id={{result.id}}", |p| Ok(format!("<{}>", p.key()))),
            Ok("page:x?id=<result.id>".to_string())
        );
        assert_eq!(
            strings(&serde_json::json!({ "a": ["x", { "b": "y" }], "n": 1 })),
            vec!["x", "y"]
        );
    }
}
