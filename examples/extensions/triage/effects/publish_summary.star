# triage/publish-summary: a finished summary run's answer, written to the
# wiki page `triage-summary` (the run's result is read back through
# `jobs.read`: the event carries only which run finished; `where:` in the
# manifest keeps it to the summarize job).

def transform(x):
    payload = x["event"]["payload"]
    if payload["outcome"] != "succeeded":
        return {"skip": "the summary didn't complete"}
    run = scope("jobs.read", {"job_run": payload["job_run"]})
    answer = run.get("result") or {}
    text = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"])
    if not text.strip():
        return {"skip": "the model answered nothing"}
    return {"commands": [{
        "name": "oxplow.knowledge.write_page",
        "input": {"slug": "triage-summary", "title": "TODO summary", "body": text},
    }]}
