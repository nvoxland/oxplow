# repo.hardcoded_secrets — string literals matching common credential shapes
# (AWS access key, GitHub/Slack token, PEM private key, Google API key) — a
# defensive-security guardrail: 0 on a clean tree, alarms the moment a secret
# lands. Scans string NODES only (ast_query), so the regex runs on small texts.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    pat = r"AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{36}|xox[baprs]-[0-9A-Za-z-]{10,}|-----BEGIN [A-Z ]*PRIVATE KEY|AIza[0-9A-Za-z_-]{35}"
    # Rust string literals.
    for f in files("**/*.rs"):
        c = 0
        for s in ast_query(f["text"], "rust", "(string_literal) @s"):
            c += len(regex_find(pat, s["text"]))
        if c > 0:
            facts.append({"measure": "repo.hardcoded_secrets", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    # TS/TSX strings.
    for tri in _ts_files():
        c = 0
        for s in ast_query(tri[1], tri[2], "(string) @s"):
            c += len(regex_find(pat, s["text"]))
        if c > 0:
            facts.append({"measure": "repo.hardcoded_secrets", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
