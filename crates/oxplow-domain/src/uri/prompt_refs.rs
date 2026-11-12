//! The references oxplow puts in a prompt, written and read by one
//! definition (`.context/agent-model.md` "Prompt references").
//!
//! ```text
//! @<path>                          a file (the agent's own picker writes it)
//! [oxplow uri <uri>]               what the person points at
//! [oxplow <uri>: "<title>" (<state>)]   a record, with its title and state
//! [oxplow page <id> row: <fields>] one row of a page node (`<page>#<node>`)
//! ```
//!
//! The writers below spell the bracketed forms; [`parse`] reads all four
//! back into the URI each one targets. The desktop's row mention
//! (`rowMention` in `viz/vizModel.ts`) is TypeScript and cannot call this:
//! `tests/fixtures/prompt_refs.json` pins its text for both languages.

use super::build;
use super::oxplow_uri::OxplowUri;

/// What a reference in a prompt points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRef {
    /// The URI it targets (a file's, a record's, a page's).
    pub target: String,
    /// The path of an `@path` reference. Only a guess at a file (`@types/node`
    /// and `@app.route` read the same), so its writer links it when the file
    /// exists; `None` for a bracketed reference, which names a URI.
    pub guessed_path: Option<String>,
}

/// `[oxplow uri <uri>]`.
pub fn uri_mention(uri: &str) -> String {
    format!("[oxplow uri {uri}]")
}

/// `[oxplow <uri>: "<title>" (<state>)]`.
pub fn item_mention(uri: &str, title: &str, state: &str) -> String {
    format!("[oxplow {uri}: \"{title}\" ({state})]")
}

/// `[oxplow page <id> row: <fields>]`, `fields` already `col=value, …`.
pub fn page_row_mention(id: &str, fields: &str) -> String {
    format!("[oxplow page {id} row: {fields}]")
}

/// The references in `text`, in order, each target once.
pub fn parse(text: &str) -> Vec<PromptRef> {
    let mut found: Vec<PromptRef> = Vec::new();
    let mut push = |target: String, guessed_path: Option<String>| {
        if !found.iter().any(|f| f.target == target) {
            found.push(PromptRef {
                target,
                guessed_path,
            });
        }
    };
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if text[i..].starts_with("[oxplow ") {
            let (target, end) = bracketed(&text[i + "[oxplow ".len()..]);
            if let Some(t) = target {
                push(t, None);
            }
            i += "[oxplow ".len() + end;
            continue;
        }
        if bytes[i] == b'@' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let rest = &text[i + 1..];
            let word = rest.split(char::is_whitespace).next().unwrap_or_default();
            let path = word.trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
            if path.contains('/')
                || path
                    .rsplit_once('.')
                    .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())
            {
                push(build::file_uri(path), Some(path.to_string()));
            }
            i += 1 + word.len();
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

/// The target of what follows `[oxplow ` and how many bytes it spans.
fn bracketed(rest: &str) -> (Option<String>, usize) {
    let word_end = |s: &str| {
        s.find(|c: char| c.is_whitespace() || c == ']')
            .unwrap_or(s.len())
    };
    let first = &rest[..word_end(rest)];
    let after = |skip: usize| rest[skip..].trim_start();
    let target = match first {
        "uri" => {
            let s = after(first.len());
            let uri = &s[..word_end(s)];
            OxplowUri::parse(uri).ok().map(|_| uri.to_string())
        }
        "page" => {
            let s = after(first.len());
            let id = &s[..word_end(s)];
            let page = id.split('#').next().unwrap_or(id);
            (!page.is_empty()).then(|| build::page_uri(page, &[]))
        }
        word => {
            let uri = word.trim_end_matches(':');
            OxplowUri::parse(uri).ok().map(|_| uri.to_string())
        }
    };
    (target, rest.find(']').map_or(rest.len(), |n| n + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_writers_are_read_back() {
        let task = "oxplow:oxplow/task?id=tsk1&provider=oxtask";
        let text = format!(
            "look at {} and {} and {}",
            uri_mention("oxplow:oxplow/commit?sha=abc1234"),
            item_mention(task, "Fix it", "todo"),
            page_row_mention("acme/list#items", "id=\"a\", n=2"),
        );
        let got: Vec<String> = parse(&text).into_iter().map(|r| r.target).collect();
        assert_eq!(
            got,
            [
                "oxplow:oxplow/commit?sha=abc1234".to_string(),
                task.to_string(),
                build::page_uri("acme/list", &[]),
            ]
        );
    }

    #[test]
    fn a_file_mention_needs_a_path_and_is_not_a_handle() {
        let got = |t: &str| parse(t).into_iter().map(|r| r.target).collect::<Vec<_>>();
        assert_eq!(got("see @src/a.rs, then"), [build::file_uri("src/a.rs")]);
        assert_eq!(got("ping @nathan about it"), Vec::<String>::new());
        assert_eq!(got("mail a@b.c"), Vec::<String>::new());
        assert_eq!(got("nothing here"), Vec::<String>::new());
    }

    /// The desktop writes a row mention in TypeScript; its text is pinned
    /// here and in `vizModel.test.ts`.
    #[test]
    fn the_desktops_row_mention_is_read() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/prompt_refs.json")).unwrap();
        let text = fixture["row_mention"]["text"].as_str().unwrap();
        let got: Vec<String> = parse(text).into_iter().map(|r| r.target).collect();
        assert_eq!(got, [build::page_uri("acme/list", &[])]);
    }
}
