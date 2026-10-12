# triage/publish-summary: a finished summary job's answer, written to the
# wiki page `triage-summary` (the job's result is read back through
# `jobs.read`: the event carries only which job finished).

def transform(x):
    payload = x["event"]["payload"]
    if payload["contribution"] != "triage/job/summarize":
        return {"skip": "another job finished"}
    if payload["outcome"] != "succeeded":
        return {"skip": "the summary didn't complete"}
    job = scope("jobs.read", {"id": payload["job_id"]})
    answer = job.get("result") or {}
    text = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"])
    if not text.strip():
        return {"skip": "the model answered nothing"}
    return {"commands": [{
        "name": "oxplow.knowledge.write_page",
        "input": {"slug": "triage-summary", "title": "TODO summary", "body": text},
    }]}
