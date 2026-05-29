import { useMemo, useState, type CSSProperties, type ReactNode } from "react";

/**
 * General-purpose file-tree renderer. Takes a flat list of items keyed
 * by path (e.g. "src/components/Foo.tsx") and groups them into
 * collapsible directory nodes. The caller provides a `renderItem` for
 * the leaf row so badges, status chips, click handlers, etc. stay the
 * caller's concern.
 *
 * Toolbar: expand-all / collapse-all / substring search. Search
 * filters the visible set to paths containing the query (case-
 * insensitive) and auto-expands their ancestors. Empty query restores
 * the user-controlled collapse state.
 *
 * Intentional non-features (yet): drag-and-drop, multi-select,
 * keyboard navigation, virtualization. Add when a caller actually
 * needs them.
 */
export interface FileTreeItem<T> {
  /** Repo-relative path, e.g. "src/components/Foo.tsx". Forward slashes. */
  path: string;
  /** Arbitrary caller data passed back to `renderItem`. */
  data: T;
}

export interface FileTreeProps<T> {
  items: FileTreeItem<T>[];
  renderItem: (item: FileTreeItem<T>) => ReactNode;
  /** Initial collapse state for directories. Default: expanded. */
  defaultCollapsed?: boolean;
  /** Optional content rendered in the toolbar between the buttons and search. */
  toolbarExtras?: ReactNode;
  /** Test id for the wrapping element. */
  testId?: string;
  /** Override placeholder shown when items is empty. */
  emptyMessage?: string;
}

interface DirNode<T> {
  kind: "dir";
  /** "" for root; otherwise the segment-joined path with no trailing slash. */
  path: string;
  /** Last path segment (display name). */
  name: string;
  children: TreeNode<T>[];
}

interface FileNode<T> {
  kind: "file";
  path: string;
  name: string;
  item: FileTreeItem<T>;
}

type TreeNode<T> = DirNode<T> | FileNode<T>;

export function FileTree<T>({
  items,
  renderItem,
  defaultCollapsed = false,
  toolbarExtras,
  testId,
  emptyMessage = "No files.",
}: FileTreeProps<T>) {
  const tree = useMemo(() => buildTree(items), [items]);
  const allDirs = useMemo(() => collectDirPaths(tree), [tree]);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() =>
    defaultCollapsed ? new Set(allDirs) : new Set(),
  );
  const [query, setQuery] = useState("");

  const visibleDirs = useMemo<ReadonlySet<string>>(() => {
    if (!query.trim()) return collapsed;
    // Search active: collapse no directories that contain a match.
    const q = query.toLowerCase();
    const matching = new Set<string>();
    for (const item of items) {
      if (item.path.toLowerCase().includes(q)) {
        // Add every ancestor path to the matching set so the row is reachable.
        let acc = "";
        for (const seg of item.path.split("/").slice(0, -1)) {
          acc = acc ? `${acc}/${seg}` : seg;
          matching.add(acc);
        }
      }
    }
    // Collapsed = everything NOT in the matching set.
    const next = new Set<string>();
    for (const dir of allDirs) {
      if (!matching.has(dir)) next.add(dir);
    }
    return next;
  }, [query, items, allDirs, collapsed]);

  function toggle(path: string) {
    if (query.trim()) return; // search drives collapse state; ignore manual toggles
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function expandAll() {
    setCollapsed(new Set());
  }

  function collapseAll() {
    setCollapsed(new Set(allDirs));
  }

  const matchPredicate = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return null;
    return (path: string) => path.toLowerCase().includes(q);
  }, [query]);

  const matchedCount = useMemo(() => {
    if (!matchPredicate) return null;
    return items.filter((it) => matchPredicate(it.path)).length;
  }, [items, matchPredicate]);

  return (
    <div data-testid={testId} style={containerStyle}>
      <div style={toolbarStyle}>
        <button
          type="button"
          onClick={expandAll}
          style={toolbarButtonStyle}
          title="Expand every directory"
          data-testid={testId ? `${testId}-expand-all` : undefined}
        >
          Expand all
        </button>
        <button
          type="button"
          onClick={collapseAll}
          style={toolbarButtonStyle}
          title="Collapse every directory"
          data-testid={testId ? `${testId}-collapse-all` : undefined}
        >
          Collapse all
        </button>
        {toolbarExtras}
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search files…"
          style={searchInputStyle}
          aria-label="Filter files"
          data-testid={testId ? `${testId}-search` : undefined}
        />
        {matchedCount !== null ? (
          <span style={matchCountStyle}>
            {matchedCount} match{matchedCount === 1 ? "" : "es"}
          </span>
        ) : null}
      </div>
      {items.length === 0 ? (
        <div style={emptyStyle}>{emptyMessage}</div>
      ) : (
        <div style={treeStyle}>
          {tree.children.map((child) => (
            <TreeRow
              key={child.path}
              node={child}
              depth={0}
              collapsed={visibleDirs}
              matchPredicate={matchPredicate}
              renderItem={renderItem}
              onToggle={toggle}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function TreeRow<T>({
  node,
  depth,
  collapsed,
  matchPredicate,
  renderItem,
  onToggle,
}: {
  node: TreeNode<T>;
  depth: number;
  collapsed: ReadonlySet<string>;
  matchPredicate: ((path: string) => boolean) | null;
  renderItem: (item: FileTreeItem<T>) => ReactNode;
  onToggle: (path: string) => void;
}) {
  if (node.kind === "file") {
    if (matchPredicate && !matchPredicate(node.path)) return null;
    return (
      <div style={{ ...rowStyle, paddingLeft: depth * 14 + 4 }}>
        {renderItem(node.item)}
      </div>
    );
  }
  const isCollapsed = collapsed.has(node.path);
  return (
    <>
      <button
        type="button"
        onClick={() => onToggle(node.path)}
        style={{ ...dirRowStyle, paddingLeft: depth * 14 + 4 }}
        data-testid={`filetree-dir-${node.path}`}
      >
        <span style={chevronStyle}>{isCollapsed ? "▸" : "▾"}</span>
        <span style={dirNameStyle}>{node.name}</span>
        <span style={dirCountStyle}>{countLeaves(node)}</span>
      </button>
      {isCollapsed
        ? null
        : node.children.map((child) => (
            <TreeRow
              key={child.path}
              node={child}
              depth={depth + 1}
              collapsed={collapsed}
              matchPredicate={matchPredicate}
              renderItem={renderItem}
              onToggle={onToggle}
            />
          ))}
    </>
  );
}

function buildTree<T>(items: FileTreeItem<T>[]): DirNode<T> {
  const root: DirNode<T> = { kind: "dir", path: "", name: "", children: [] };
  // Stable ordering: directories first, then files, alphabetically.
  // We get that for free by inserting in path-sorted order and sorting
  // each level afterwards.
  const sorted = [...items].sort((a, b) => a.path.localeCompare(b.path));
  for (const item of sorted) {
    insertItem(root, item);
  }
  sortDir(root);
  return root;
}

function insertItem<T>(root: DirNode<T>, item: FileTreeItem<T>): void {
  const segs = item.path.split("/").filter((s) => s.length > 0);
  if (segs.length === 0) return;
  let parent = root;
  for (let i = 0; i < segs.length - 1; i++) {
    const segPath = segs.slice(0, i + 1).join("/");
    let dir = parent.children.find(
      (c): c is DirNode<T> => c.kind === "dir" && c.path === segPath,
    );
    if (!dir) {
      dir = { kind: "dir", path: segPath, name: segs[i]!, children: [] };
      parent.children.push(dir);
    }
    parent = dir;
  }
  const fileName = segs[segs.length - 1]!;
  parent.children.push({ kind: "file", path: item.path, name: fileName, item });
}

function sortDir<T>(dir: DirNode<T>): void {
  dir.children.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  for (const child of dir.children) {
    if (child.kind === "dir") sortDir(child);
  }
}

function collectDirPaths<T>(dir: DirNode<T>): string[] {
  const out: string[] = [];
  function walk(d: DirNode<T>) {
    if (d.path) out.push(d.path);
    for (const c of d.children) {
      if (c.kind === "dir") walk(c);
    }
  }
  walk(dir);
  return out;
}

function countLeaves<T>(node: TreeNode<T>): number {
  if (node.kind === "file") return 1;
  let n = 0;
  for (const c of node.children) n += countLeaves(c);
  return n;
}

const containerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
  gap: 4,
};

const toolbarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "4px 0",
  fontSize: "var(--text-sm)",
};

const toolbarButtonStyle: CSSProperties = {
  fontSize: "var(--text-sm)",
  padding: "3px 8px",
  borderRadius: 3,
  border: "1px solid var(--border-subtle)",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  cursor: "pointer",
};

const searchInputStyle: CSSProperties = {
  flex: 1,
  fontSize: "var(--text-sm)",
  padding: "3px 8px",
  borderRadius: 3,
  border: "1px solid var(--border-subtle)",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
};

const matchCountStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
  whiteSpace: "nowrap",
};

const treeStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 1,
};

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  minHeight: 22,
};

const dirRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 4,
  fontSize: "var(--text-sm)",
  color: "var(--text-secondary)",
  background: "transparent",
  border: "none",
  cursor: "pointer",
  textAlign: "left",
  padding: "2px 4px",
};

const chevronStyle: CSSProperties = {
  width: 16,
  display: "inline-block",
  fontSize: 16,
  color: "var(--text-secondary)",
};

const dirNameStyle: CSSProperties = {
  fontWeight: "var(--weight-medium)",
};

const dirCountStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  marginLeft: 4,
};

const emptyStyle: CSSProperties = {
  padding: "8px 4px",
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
};
