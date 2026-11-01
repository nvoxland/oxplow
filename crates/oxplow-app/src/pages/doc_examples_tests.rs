//! The page examples in the `oxplow-extension` skill's references parse,
//! so the docs cannot drift from the format. A fenced block is checked as
//! a page file when its info string is `yaml page`, and as an inline page
//! when it is an `oxplow-page` block.

use super::schema::{parse_inline_page, parse_page, PageKind};

/// `(line of the opening fence, is inline, text)` of each marked block.
fn marked_blocks(markdown: &str) -> Vec<(usize, bool, String)> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let info = lines[i].trim().trim_start_matches('`');
        let inline = lines[i].trim() == "```oxplow-page";
        if inline || (lines[i].trim().starts_with("```") && info == "yaml page") {
            let start = i + 1;
            let mut end = start;
            while end < lines.len() && lines[end].trim() != "```" {
                end += 1;
            }
            // A block inside a list item is indented as a whole.
            let indent = lines[start..end]
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(0);
            let text: Vec<&str> = lines[start..end]
                .iter()
                .map(|l| l.get(indent..).unwrap_or(""))
                .collect();
            out.push((i + 1, inline, text.join("\n")));
            i = end;
        }
        i += 1;
    }
    out
}

#[test]
fn every_marked_page_example_in_the_page_references_parses() {
    let skill = oxplow_agent_text::core_text()
        .skills
        .into_iter()
        .find(|s| s.name == "oxplow-extension")
        .expect("the oxplow-extension skill");
    let mut checked = 0;
    let mut failures = Vec::new();
    for reference in &skill.references {
        for (line, inline, text) in marked_blocks(&reference.body) {
            checked += 1;
            let parsed = if inline {
                parse_inline_page(&text).map(|_| ())
            } else {
                parse_page(
                    "acme",
                    "acme",
                    "example",
                    "pages/example.yaml",
                    &text,
                    PageKind::Page,
                )
                .map(|_| ())
            };
            if let Err(e) = parsed {
                failures.push(format!("references/{}.md:{line}: {e}", reference.name));
            }
        }
    }
    assert!(checked >= 5, "only {checked} marked page examples");
    assert_eq!(failures, Vec::<String>::new());
}
