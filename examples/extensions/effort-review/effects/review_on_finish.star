# effort-review/review-on-finish: a finished effort with a work item gets a
# review (`effort.review` holds the prompt; a second delivery of the same
# event asks for the same job, not another).

def transform(x):
    payload = x["event"]["payload"]
    if not payload.get("work_item"):
        return {"skip": "the effort has no work item to comment on"}
    return {"commands": [{"name": "effort_review.effort.review", "input": {"effort": payload["effort"]}}]}
