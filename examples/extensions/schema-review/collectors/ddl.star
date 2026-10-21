# The DDL in each migration file (`migrations/**.sql`) of the snapshot:
# one fact per kind of statement it holds, counted.

KINDS = {"CREATE TABLE": "create", "ALTER TABLE": "alter", "DROP TABLE": "drop"}

def transform(input):
    facts = []
    for f in files("migrations/**/*.sql"):
        text = f["text"].upper()
        for statement, kind in KINDS.items():
            n = text.count(statement)
            if n:
                facts.append({
                    "measure": "schema_review.ddl",
                    "value": n,
                    "subject": uri.build("oxplow/file", path = f["path"]),
                    "path": f["path"],
                    "dims": {"schema_review.ddl_kind": kind},
                })
    return {"facts": facts}
