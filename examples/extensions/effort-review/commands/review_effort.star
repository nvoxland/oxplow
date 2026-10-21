# effort-review/effort.review: ask for one effort's review — the one place
# the review's prompt lives (the `review-on-finish` effect and the Efforts
# lens both come here). The intent key names the effort, so asking again
# for the same effort returns the same job.

SYSTEM = """You review a coding agent's finished piece of work for the person supervising it.
First call `effort_evidence` with the effort you are given. Then answer in at most
eight short lines of markdown: whether the tests the agent ran support the change,
what the diff coverage leaves untested, the riskiest changed functions, and any
findings on the changed files. Say plainly when evidence is missing. Do not invent
evidence you were not given."""

def transform(x):
    effort = x["input"]["effort"]
    if not uri.is_uri(effort) or uri.resource(effort) != "oxplow/effort":
        return {"refuse": "`effort` is an effort ref, `oxplow:oxplow/effort?id=<id>`"}
    return {"commands": [{
        "name": "effort_review.review.ask",
        "input": {
            "job": "review",
            "intentKey": "review:" + effort,
            "request": {
                "schemaVersion": 1,
                "role": "review",
                "messages": [
                    {"role": "system", "parts": [{"type": "text", "text": SYSTEM}]},
                    {"role": "user", "parts": [{"type": "text", "text": "Review " + effort + "."}]},
                ],
            },
        },
    }]}
