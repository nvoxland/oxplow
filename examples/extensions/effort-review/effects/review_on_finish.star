# effort_review/review-on-finish: a finished effort with a task gets a
# review (`effort.review` holds the prompt; a second delivery of the same
# event asks for the same job, not another).

def transform(x):
    payload = x["event"]["payload"]
    if not payload.get("task"):
        return {"skip": "the effort has no task to comment on"}
    return {"commands": [{"name": "effort_review.effort.review", "input": {"effort": payload["effort"]}}]}
