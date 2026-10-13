# effort-review/publish-review: a review job's answer, commented on the
# effort's work item. The event carries only which job ended; the job's
# intent key (`review:<effort ref>`) says which effort it was about.

PREFIX = "review:effort:eff"

def transform(x):
    payload = x["event"]["payload"]
    if payload["contribution"] != "effort-review/job/review":
        return {"skip": "another job ended"}
    if payload["outcome"] not in ("succeeded", "cancelled"):
        return {"skip": "the review didn't finish (" + payload["outcome"] + ")"}
    job = scope("jobs.read", {"id": payload["job_id"]})
    key = job.get("intentKey") or ""
    if not key.startswith(PREFIX):
        return {"skip": "not an effort review: " + key}
    efforts = scope("sql.read", {
        "sql": "SELECT work_item FROM v_effort WHERE id = :id",
        "params": {"id": int(key[len(PREFIX):])},
    })
    if not efforts or not efforts[0]["work_item"]:
        return {"skip": "no work item for " + key[len("review:"):]}
    answer = job.get("result") or {}
    text = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"]).strip()
    if not text:
        return {"skip": "the model wrote nothing"}
    if payload["outcome"] == "cancelled":
        text += "\n\n_The review was cancelled after " + str(len(text.split())) + " words._"
    return {"commands": [{
        "name": "oxplow.work_item.comment",
        "input": {"ref": efforts[0]["work_item"], "body": "**Review of " + key[len("review:"):] + "**\n\n" + text},
    }]}
