# Claude Code's memories of this project are the markdown files of the
# `memory/` folder in its own data folder for the project (root
# `agent:claude`). On start and whenever one changes, state them all as the
# rows of the knowledge model's `claude-memory` store (a snapshot: a file
# that is gone is a memory that is gone). MEMORY.md is Claude Code's index
# of them: no page, but where each memory's title comes from.

ROOT = "agent:claude"
FOLDER = "memory"
STORE = "claude-memory"
INDEX = "MEMORY.md"

# A memory file bigger than this is skipped (and said so), as is one whose
# bytes are not UTF-8: one such file never fails the whole snapshot.
MAX_BYTES = 1048576

def unquote(v):
    v = v.strip()
    if len(v) >= 2 and v[0] == v[-1] and v[0] in ["'", "\""]:
        return v[1:-1]
    return v

def split_frontmatter(text):
    """The frontmatter's keys (a top-level `key: value`, and `type` under
    `metadata:` as `metadata.type`) and the text after it."""
    meta = {}
    if not text.startswith("---\n"):
        return meta, text
    lines = text.split("\n")
    block = ""
    for i in range(1, len(lines)):
        line = lines[i]
        if line.strip() == "---":
            return meta, "\n".join(lines[i + 1:])
        if line.strip() == "" or line.lstrip().startswith("#"):
            continue
        indented = line.startswith(" ") or line.startswith("\t")
        if ":" not in line:
            continue
        key, value = line.split(":", 1)
        key = key.strip()
        if not indented:
            block = key
            if value.strip() != "":
                meta[key] = unquote(value)
        elif block != "":
            meta[block + "." + key] = unquote(value)
    # No closing line: it was never frontmatter.
    return {}, text

def index_titles(text):
    """MEMORY.md's `- [Title](file.md) — hook` lines: file id → title."""
    titles = {}
    for line in text.split("\n"):
        start = line.find("[")
        mid = line.find("](", start + 1)
        if start < 0 or mid < 0:
            continue
        end = line.find(")", mid + 2)
        if end < 0:
            continue
        target = line[mid + 2:end].strip()
        if target.endswith(".md") and "/" not in target:
            titles[target[:-3]] = line[start + 1:mid].strip()
    return titles

def bare(target):
    for c in [":", "/", "#", "|", ".", " "]:
        if c in target:
            return False
    return target != ""

def resolve(target, ids, names):
    """The id of the memory `target` names: by its `name`, by its file, or
    with `_` and `-` swapped; else `target` itself (a dangling link inside
    the store, never another store's page)."""
    for t in [target, target.replace("_", "-"), target.replace("-", "_")]:
        if t in names:
            return names[t]
        if t in ids:
            return t
    return target

def with_store_links(text, ids, names):
    """Each bare `[[x]]` as `[[claude-memory:<id>]]`; a link to a file or
    another store's page is kept as written."""
    out = []
    rest = text
    for _ in range(len(text)):
        start = rest.find("[[")
        if start < 0:
            break
        end = rest.find("]]", start + 2)
        if end < 0:
            break
        target = rest[start + 2:end]
        out.append(rest[:start])
        if bare(target):
            out.append("[[" + STORE + ":" + resolve(target, ids, names) + "]]")
        else:
            out.append(rest[start:end + 2])
        rest = rest[end + 2:]
    out.append(rest)
    return "".join(out)

def transform(x):
    problems = []
    memories = []
    titles = {}
    depth = len(FOLDER.split("/")) + 1
    for f in scope("files.list", {"root": ROOT, "path": FOLDER, "glob": "*.md"})["files"]:
        parts = f["path"].split("/")
        # Only a file directly in the folder is a memory.
        if len(parts) != depth:
            continue
        name = parts[-1]
        if f["size"] > MAX_BYTES:
            problems.append("`%s` is bigger than %d bytes: skipped" % (f["path"], MAX_BYTES))
            continue
        read = scope("files.read", {"root": ROOT, "path": f["path"], "lossy": True})
        if read["lossy"]:
            problems.append("`%s` is not UTF-8 text: skipped" % f["path"])
            continue
        if name == INDEX:
            titles = index_titles(read["body"])
            continue
        meta, text = split_frontmatter(read["body"])
        memories.append((name[:-3], meta, text, f["modified_at"]))
    ids = {}
    names = {}
    for (id, meta, _, _) in memories:
        ids[id] = True
        if meta.get("name"):
            names[meta["name"]] = id
    rows = []
    for (id, meta, text, modified_at) in memories:
        text = with_store_links(text.lstrip("\n"), ids, names)
        description = meta.get("description", "")
        rows.append({
            "id": id,
            "title": titles.get(id) or meta.get("name") or id,
            "body": description + "\n\n" + text if description else text,
            "kind": meta.get("type") or meta.get("metadata.type") or "memory",
            "author": "Claude Code",
            "updated_at": modified_at,
        })
    return {
        "models": [{"model": "oxplow/knowledge_record", "store": STORE, "snapshot": rows}],
        "problems": problems,
    }
