//! The pages the required bundled extension declares as built-in
//! components (`pages: [{ component: "oxplow:<name>" }]`): core React the
//! kit can't express yet. A stepping stone, not a kind of page: each one
//! leaves this table when its page becomes a lens or a custom page, and
//! nothing joins it. `fixtures/built_in_pages.json` pins the table so a new
//! name fails review; the desktop's registry (`pages/builtInPages.tsx`)
//! renders exactly these.

/// The built-in page components, by name.
pub const BUILT_IN_PAGES: &[&str] = &[
    "alerts",
    "backlog",
    "board",
    "comments",
    "custom-dashboard",
    "diff",
    "diff-view",
    "duplicate-block",
    "explore-data",
    "external-url",
    "files",
    "git-dashboard",
    "metrics-recorded",
    "settings",
    "tasks",
    "terminal",
    "uncommitted-changes",
];

/// The built-in named by a manifest's `component: "oxplow:<name>"`.
pub fn built_in(component: &str) -> Option<&'static str> {
    let name = component.strip_prefix("oxplow:")?;
    BUILT_IN_PAGES.iter().copied().find(|b| *b == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table only shrinks: a page leaves it when it becomes a lens or a
    /// custom page. A name missing from the golden is a new built-in page,
    /// which the epic doesn't allow; a golden name missing from the table is
    /// a page that left — take it out of the golden too.
    #[test]
    fn the_built_in_pages_only_shrink() {
        let golden: Vec<String> =
            serde_json::from_str(include_str!("../fixtures/built_in_pages.json"))
                .expect("the golden is a JSON list");
        let added: Vec<&&str> = BUILT_IN_PAGES
            .iter()
            .filter(|b| !golden.iter().any(|g| g == **b))
            .collect();
        assert!(
            added.is_empty(),
            "new built-in pages {added:?}: a page is a lens or a custom page, not core code"
        );
        let left: Vec<&String> = golden
            .iter()
            .filter(|g| !BUILT_IN_PAGES.contains(&g.as_str()))
            .collect();
        assert!(
            left.is_empty(),
            "{left:?} left the table: take them out of fixtures/built_in_pages.json"
        );
    }

    #[test]
    fn a_component_names_a_built_in_by_its_oxplow_prefix() {
        assert_eq!(built_in("oxplow:tasks"), Some("tasks"));
        assert_eq!(built_in("tasks"), None);
        assert_eq!(built_in("oxplow:nope"), None);
    }
}
