//! Where one page file names another, and what each place may name.
//!
//! A page's params, nodes, loaders and links, an extension's panels, slot
//! mounts and replacements all refer to page files by id. Each place is a
//! [`RefSite`]: it says what kind the target must be — a query for a
//! param's options, a part or a page for something drawn inside a host, a
//! page for a link — and what shape of rows it needs. The loader's checks,
//! the slug qualifier and the cross-extension pass all read this one
//! table, so a new place is one variant and one arm of [`for_each_ref`].

use crate::pages::schema::{Loader, Node, NodeKind, Page, PageKind, SeeAlsoDecl};
use crate::pages::spec::{FormField, FormFieldSpec, PageAction, PageParam};

/// Which kinds a place may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// Rows only: a query.
    Query,
    /// Something drawn inside a host: a part, or a page (a dashboard
    /// embeds pages).
    Hosted,
    /// A link a person follows: a page.
    Page,
    /// A loader borrowed from any kind.
    Any,
}

impl RefKind {
    pub fn allows(self, kind: PageKind) -> bool {
        match self {
            RefKind::Query => kind == PageKind::Query,
            RefKind::Hosted => kind != PageKind::Query,
            RefKind::Page => kind == PageKind::Page,
            RefKind::Any => true,
        }
    }

    /// What the place takes, in words: for an error.
    pub fn words(self) -> &'static str {
        match self {
            RefKind::Query => "a query (queries/)",
            RefKind::Hosted => "a part (parts/) or a page (pages/)",
            RefKind::Page => "a page (pages/)",
            RefKind::Any => "a page, a part or a query",
        }
    }
}

/// The columns a place reads from the rows it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    None,
    /// `value`, `label`, optional `group`: a choice's options.
    Options,
    /// `name`, `title`, `kind`: a form field's sub-fields.
    Fields,
}

/// One place a page file is named from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefSite {
    /// A choice param's `options_from` (the page's, or a viz node's).
    OptionsFrom {
        param: String,
    },
    /// A `see_also` link.
    SeeAlso,
    /// A column's `expand`.
    Expand {
        node: String,
        column: String,
    },
    /// A form field's `from`: its sub-fields.
    FormFrom {
        node: String,
        field: String,
    },
    /// A form's or a form action's `choices` for a field.
    FormChoices {
        node: String,
        field: String,
    },
    /// An `embed` node.
    Embed {
        node: String,
    },
    /// A `{ page, loader }` loader.
    PageLoader {
        loader: String,
    },
    /// A panel's parts.
    PanelBody,
    PanelCollapsed,
    PanelCount,
    PanelBadge,
    /// A `ui.mounts` entry at a slot.
    SlotMount,
    /// A `ui.mounts` entry at a replaceable component.
    Replacement,
}

impl RefSite {
    /// What kind the place may name.
    pub fn needs(&self) -> RefKind {
        match self {
            RefSite::OptionsFrom { .. }
            | RefSite::FormChoices { .. }
            | RefSite::FormFrom { .. } => RefKind::Query,
            RefSite::SeeAlso => RefKind::Page,
            RefSite::PageLoader { .. } => RefKind::Any,
            RefSite::Expand { .. }
            | RefSite::Embed { .. }
            | RefSite::PanelBody
            | RefSite::PanelCollapsed
            | RefSite::PanelCount
            | RefSite::PanelBadge
            | RefSite::SlotMount
            | RefSite::Replacement => RefKind::Hosted,
        }
    }

    /// The columns the place reads.
    pub fn shape(&self) -> Shape {
        match self {
            RefSite::OptionsFrom { .. } | RefSite::FormChoices { .. } => Shape::Options,
            RefSite::FormFrom { .. } => Shape::Fields,
            _ => Shape::None,
        }
    }

    /// Whether the place may name the page it is on: a form's own loader
    /// may list a field's choices or sub-fields.
    pub fn self_ok(&self) -> bool {
        matches!(self, RefSite::FormChoices { .. } | RefSite::FormFrom { .. })
    }

    /// The place, in words: the start of an error about it.
    pub fn what(&self) -> String {
        match self {
            RefSite::OptionsFrom { param } => format!("param `{param}`: `options_from`"),
            RefSite::SeeAlso => "see_also".to_string(),
            RefSite::Expand { node, column } => {
                format!("node `{node}`: column `{column}`: `expand`")
            }
            RefSite::FormFrom { node, field } => {
                format!("node `{node}`: form field `{field}`: `from`")
            }
            RefSite::FormChoices { node, field } => {
                format!("node `{node}`: `choices` for `{field}`")
            }
            RefSite::Embed { node } => format!("node `{node}`: `embed`"),
            RefSite::PageLoader { loader } => format!("loader `{loader}`: `page`"),
            RefSite::PanelBody => "its body".to_string(),
            RefSite::PanelCollapsed => "its collapsed".to_string(),
            RefSite::PanelCount => "its count".to_string(),
            RefSite::PanelBadge => "its badge".to_string(),
            RefSite::SlotMount => "the mount".to_string(),
            RefSite::Replacement => "the replacement".to_string(),
        }
    }
}

/// `<id>` and the `#<loader>` a reference may end in.
pub fn split(target: &str) -> (&str, Option<&str>) {
    match target.split_once('#') {
        Some((id, loader)) => (id, Some(loader)),
        None => (target, None),
    }
}

/// Why `target` can't be what `site` names: it is the wrong kind.
pub fn kind_problem(site: &RefSite, target: &Page) -> Option<String> {
    let needs = site.needs();
    if needs.allows(target.kind) {
        return None;
    }
    Some(format!(
        "{} names `{}`, {} ({}): it takes {}",
        site.what(),
        target.id,
        target.kind.article(),
        target.file(),
        needs.words()
    ))
}

/// Every reference `page` holds, each with its place, for `f` to read or
/// rewrite: the one traversal.
pub fn for_each_ref(page: &mut Page, mut f: impl FnMut(&RefSite, &mut String)) {
    fn params(params: &mut [PageParam], f: &mut impl FnMut(&RefSite, &mut String)) {
        for p in params {
            if let Some(from) = &mut p.options_from {
                f(
                    &RefSite::OptionsFrom {
                        param: p.name.clone(),
                    },
                    from,
                );
            }
        }
    }
    fn actions(node: &str, actions: &mut [PageAction], f: &mut impl FnMut(&RefSite, &mut String)) {
        for a in actions {
            for (field, target) in &mut a.choices {
                f(
                    &RefSite::FormChoices {
                        node: format!("{node}: action `{}`", a.id),
                        field: field.clone(),
                    },
                    target,
                );
            }
        }
    }
    fn walk(node: &mut Node, f: &mut impl FnMut(&RefSite, &mut String)) {
        let id = node.id.clone();
        match &mut node.kind {
            NodeKind::Stack { children }
            | NodeKind::Grid { children }
            | NodeKind::Switch { children, .. } => {
                for c in children {
                    walk(c, f);
                }
            }
            NodeKind::Embed { page, .. } => f(&RefSite::Embed { node: id }, page),
            NodeKind::Viz { config, .. } => {
                params(&mut config.params, f);
                for c in &mut config.columns {
                    if let Some(to) = &mut c.expand {
                        f(
                            &RefSite::Expand {
                                node: id.clone(),
                                column: c.key.clone(),
                            },
                            to,
                        );
                    }
                }
                if let Some(form) = &mut config.form {
                    for field in form.fields.iter_mut().flatten() {
                        if let FormField::Spec(FormFieldSpec {
                            name,
                            from: Some(from),
                            ..
                        }) = field
                        {
                            f(
                                &RefSite::FormFrom {
                                    node: id.clone(),
                                    field: name.clone(),
                                },
                                from,
                            );
                        }
                    }
                    for (field, target) in &mut form.choices {
                        f(
                            &RefSite::FormChoices {
                                node: id.clone(),
                                field: field.clone(),
                            },
                            target,
                        );
                    }
                }
                actions(&id, &mut config.actions, f);
            }
            NodeKind::Slot { .. } | NodeKind::Markdown { .. } | NodeKind::Component { .. } => {}
        }
    }
    params(&mut page.params, &mut f);
    for entry in &mut page.see_also {
        if let SeeAlsoDecl::Page { page: to, .. } = entry {
            f(&RefSite::SeeAlso, to);
        }
    }
    for decl in &mut page.loaders {
        if let Loader::Page { page: to, .. } = &mut decl.loader {
            f(
                &RefSite::PageLoader {
                    loader: decl.name.clone(),
                },
                to,
            );
        }
    }
    actions("page", &mut page.actions, &mut f);
    walk(&mut page.view, &mut f);
    if let Some(rail) = &mut page.rail {
        walk(rail, &mut f);
    }
}

/// Every reference `page` holds, each with its place and target.
pub fn refs(page: &Page) -> Vec<(RefSite, String)> {
    let mut copy = page.clone();
    let mut out = Vec::new();
    for_each_ref(&mut copy, |site, target| {
        out.push((site.clone(), target.clone()))
    });
    out
}
