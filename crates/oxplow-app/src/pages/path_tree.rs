//! A `tree` viz over paths (`tree.path`): each row a file, its folders made
//! from its path, and a folder holding one folder and nothing else joined
//! with it (`.agents/skills/`). `tree.status` is core's change vocabulary
//! (`FileStatus`): each row's mark, and a made folder's the marks of what
//! is under it. The window builds the same tree (`viz/vizModel.ts`
//! `pathTreeNodes`), from the same cases.

/// One node: what it reads as, the row it is (none for a made folder), its
/// marks, and its children (folders first, then files, each by name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathNode {
    /// Its name, or a joined chain's names (`skills/x`).
    pub label: String,
    /// The full path (a joined chain's deepest folder's).
    pub path: String,
    /// The result row it is; a made folder has none.
    pub row: Option<usize>,
    /// Its status marks, in [`MARKS`] order.
    pub marks: Vec<&'static Mark>,
    pub children: Vec<PathNode>,
}

impl PathNode {
    /// A folder: it holds something, or it was made for what it holds.
    pub fn is_folder(&self) -> bool {
        !self.children.is_empty()
    }
}

/// One status mark: the status it is, its letter, its tone.
#[derive(Debug, PartialEq, Eq)]
pub struct Mark {
    pub status: &'static str,
    pub letter: &'static str,
    pub tone: &'static str,
}

/// Every status a mark is drawn for, in the order a folder lists them.
pub const MARKS: [Mark; 6] = [
    Mark {
        status: "added",
        letter: "A",
        tone: "success",
    },
    Mark {
        status: "modified",
        letter: "M",
        tone: "warning",
    },
    Mark {
        status: "deleted",
        letter: "D",
        tone: "danger",
    },
    Mark {
        status: "renamed",
        letter: "R",
        tone: "accent",
    },
    Mark {
        status: "untracked",
        letter: "U",
        tone: "success",
    },
    Mark {
        status: "conflicted",
        letter: "C",
        tone: "danger",
    },
];

/// The mark for `status`; none for a value that isn't one.
pub fn mark(status: &str) -> Option<&'static Mark> {
    MARKS.iter().find(|m| m.status == status)
}

/// The tree of `paths` (`(path, status)` per row; an empty path is left
/// out). A path that is also a folder of another keeps its row.
pub fn build(paths: &[(String, Option<String>)]) -> Vec<PathNode> {
    #[derive(Default)]
    struct Dir {
        row: Option<usize>,
        children: std::collections::BTreeMap<String, Dir>,
    }
    let mut root = Dir::default();
    for (i, (path, _)) in paths.iter().enumerate() {
        let path = path.trim_matches('/');
        if path.is_empty() {
            continue;
        }
        let mut at = &mut root;
        for seg in path.split('/') {
            at = at.children.entry(seg.to_string()).or_default();
        }
        at.row.get_or_insert(i);
    }
    fn nodes(dir: Dir, prefix: &str, paths: &[(String, Option<String>)]) -> Vec<PathNode> {
        let mut out: Vec<PathNode> = dir
            .children
            .into_iter()
            .map(|(name, d)| {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                let row = d.row;
                let children = nodes(d, &path, paths);
                let own = row.and_then(|r| paths[r].1.as_deref()).and_then(mark);
                let marks = match own {
                    Some(m) => vec![m],
                    None => MARKS
                        .iter()
                        .filter(|m| children.iter().any(|c| c.marks.contains(m)))
                        .collect(),
                };
                let node = PathNode {
                    label: name,
                    path,
                    row,
                    marks,
                    children,
                };
                join(node)
            })
            .collect();
        // Folders first, then files; each by name (the map's order).
        out.sort_by_key(|n| !n.is_folder());
        out
    }
    nodes(root, "", paths)
}

/// A made folder whose one child is a made folder reads as one row.
fn join(mut node: PathNode) -> PathNode {
    while node.row.is_none() && node.children.len() == 1 {
        let only = &node.children[0];
        if only.row.is_some() || !only.is_folder() {
            break;
        }
        let only = node.children.remove(0);
        node.label = format!("{}/{}", node.label, only.label);
        node.path = only.path;
        node.marks = only.marks;
        node.children = only.children;
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(paths: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
        paths
            .iter()
            .map(|(p, s)| (p.to_string(), (!s.is_empty()).then(|| s.to_string())))
            .collect()
    }

    /// The tree as text: `label [marks] (row)`, indented.
    fn text(nodes: &[PathNode], depth: usize, out: &mut Vec<String>) {
        for n in nodes {
            let marks: String = n.marks.iter().map(|m| m.letter).collect();
            let row = n.row.map_or(String::new(), |r| format!(" #{r}"));
            out.push(format!("{}{} {marks}{row}", "  ".repeat(depth), n.label));
            text(&n.children, depth + 1, out);
        }
    }

    fn shown(paths: &[(&str, &str)]) -> Vec<String> {
        let mut out = Vec::new();
        text(&build(&rows(paths)), 0, &mut out);
        out
    }

    /// Folders are made for the files, folders first, each by name; a
    /// made folder's marks are the distinct marks under it, in order.
    #[test]
    fn folders_are_made_and_carry_the_marks_under_them() {
        assert_eq!(
            shown(&[
                ("src/ui/a.rs", "modified"),
                ("src/new.rs", "added"),
                ("old.rs", "deleted"),
                ("README.md", "")
            ]),
            vec![
                "src AM",
                "  ui M",
                "    a.rs M #0",
                "  new.rs A #1",
                "README.md  #3",
                "old.rs D #2",
            ]
        );
    }

    /// A made folder holding one folder and nothing else joins with it;
    /// one holding a file, or two things, does not.
    #[test]
    fn a_chain_of_lone_folders_is_one_row() {
        assert_eq!(
            shown(&[
                (".agents/skills/x/SKILL.md", "untracked"),
                ("a/b/c.rs", "added"),
                ("a/d.rs", "added")
            ]),
            vec![
                ".agents/skills/x U",
                "  SKILL.md U #0",
                "a A",
                "  b A",
                "    c.rs A #1",
                "  d.rs A #2",
            ]
        );
    }

    /// A folder that is a row of its own keeps it and its own status, and
    /// is never joined; an unknown status draws no mark.
    #[test]
    fn a_folder_row_keeps_its_row_and_status() {
        assert_eq!(
            shown(&[("lib", "renamed"), ("lib/x/y.rs", "weird")]),
            vec!["lib R #0", "  x ", "    y.rs  #1"]
        );
    }
}
