# Forge links (Open In ▸ GitHub, GitLab, Bitbucket; Copy Revision Link): a
# file of the stream's worktree on the forge its repository is on, at the
# stream's head commit, with the lines its URI names (`#lines=10-20`).
#
# `transform` composes `oxplow.window.open_url` (a forge's command) or
# `oxplow.window.copy_text` (Copy Revision Link) with that URL. The
# `about_if` functions (`on_github`, …, `on_forge`) offer each only when the
# repository has a remote on that forge (`v_vcs_remote`).

FORGES = {
    "github": "GitHub",
    "gitlab": "GitLab",
    "bitbucket": "Bitbucket",
}

def _host_and_repo(url):
    """`git@host:owner/repo.git`, `ssh://git@host/owner/repo` or
    `https://[user@]host[:port]/owner/repo(.git)`: (host, owner/repo)."""
    u = url.strip()
    if "://" in u:
        rest = u.split("://", 1)[1]
        authority, _, path = rest.partition("/")
        host = authority.split("@")[-1].split(":")[0]
    elif "@" in u and ":" in u:
        host_part, _, path = u.partition(":")
        host = host_part.split("@")[-1]
    else:
        return None
    if path.endswith(".git"):
        path = path[:-4]
    path = path.strip("/")
    if not host or not path:
        return None
    return (host.lower(), path)

def _forge_of(host):
    for forge in ["github", "gitlab", "bitbucket"]:
        if forge in host:
            return forge
    return None

def _remotes():
    """The repository's forge remotes, origin first: [(forge, host, repo)]."""
    rows = scope("sql.read", {"sql": "SELECT name, url FROM v_vcs_remote ORDER BY name"})
    out = []
    for r in sorted(rows, key = lambda r: 0 if r["name"] == "origin" else 1):
        parsed = _host_and_repo(r["url"])
        if parsed == None:
            continue
        forge = _forge_of(parsed[0])
        if forge != None:
            out.append((forge, parsed[0], parsed[1]))
    return out

def _remote(forge):
    for r in _remotes():
        if forge == None or r[0] == forge:
            return r
    return None

def _file_of(uri):
    """The file URI's path (as written, still encoded) and its lines."""
    base, _, view = uri.partition("#")
    path = None
    for part in base.partition("?")[2].split("&"):
        if part.startswith("path="):
            path = part[len("path="):]
    lines = None
    for part in view.split("&"):
        if part.startswith("lines="):
            bounds = part[len("lines="):].split("-")
            lines = (bounds[0], bounds[-1])
    return (path, lines)

def _anchor(forge, lines):
    if lines == None:
        return ""
    start, end = lines[0], lines[1]
    if forge == "bitbucket":
        return "#lines-%s" % start if start == end else "#lines-%s:%s" % (start, end)
    if start == end:
        return "#L%s" % start
    if forge == "gitlab":
        return "#L%s-%s" % (start, end)
    return "#L%s-L%s" % (start, end)

def _url(forge, host, repo, sha, path, lines):
    blob = {"github": "blob", "gitlab": "-/blob", "bitbucket": "src"}[forge]
    return "https://%s/%s/%s/%s/%s%s" % (host, repo, blob, sha, path, _anchor(forge, lines))

def _head(stream):
    rows = scope("sql.read", {"sql": "SELECT head_sha FROM v_stream WHERE uri = :s", "params": {"s": "oxplow:oxplow/stream?id=" + stream}})
    return rows[0]["head_sha"] if rows else None

def transform(x):
    i = x["input"]
    forge = i.get("forge")
    remote = _remote(forge)
    if remote == None:
        return {"refuse": "the repository has no %s remote" % (FORGES[forge] if forge else "GitHub, GitLab or Bitbucket")}
    path, lines = _file_of(i["uri"])
    if not path:
        return {"refuse": "`%s` names no file" % i["uri"]}
    sha = _head(i["stream"])
    if not sha:
        return {"refuse": "stream `%s` has no commit" % i["stream"]}
    url = _url(remote[0], remote[1], remote[2], sha, path, lines)
    if i.get("copy"):
        return {"commands": [{"name": "oxplow.window.copy_text", "input": {"text": url}}], "result": {"url": url}}
    return {"commands": [{"name": "oxplow.window.open_url", "input": {"url": url}}], "result": {"url": url}}

# The `about_if` functions: offered only where the repository is on it.
def on_github(link):
    return _remote("github") != None

def on_gitlab(link):
    return _remote("gitlab") != None

def on_bitbucket(link):
    return _remote("bitbucket") != None

def on_forge(link):
    return _remote(None) != None
