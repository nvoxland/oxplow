# effort-review/publish-review: a review's answer, commented on the effort's
# work item. The event carries only which run of the review job ended
# (`where:` in the manifest keeps it to this job); the run's intent key,
# `review:<effort ref>`, says which effort it was about.

PREFIX = "review:"

def transform(x):
    payload = x["event"]["payload"]
    if payload["outcome"] not in ("succeeded", "cancelled"):
        return {"skip": "the review didn't finish (" + payload["outcome"] + ")"}
    run = scope("jobs.read", {"job_run": payload["job_run"]})
    key = run.get("intentKey") or ""
    effort = key[len(PREFIX):] if key.startswith(PREFIX) else ""
    if not uri.is_uri(effort) or uri.type(effort) != "effort":
        return {"skip": "not an effort review: " + key}
    efforts = scope("sql.read", {
        "sql": "SELECT work_item FROM v_effort WHERE ref = :effort",
        "params": {"effort": effort},
    })
    if not efforts or not efforts[0]["work_item"]:
        return {"skip": "no work item for " + effort}
    answer = run.get("result") or {}
    text = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"]).strip()
    if not text:
        return {"skip": "the model wrote nothing"}
    if payload["outcome"] == "cancelled":
        text += "\n\n_The review was cancelled after " + str(len(text.split())) + " words._"
    return {"commands": [{
        "name": "oxplow.work_item.comment",
        "input": {"ref": efforts[0]["work_item"], "body": "**Review of " + effort + "**\n\n" + text},
    }]}
