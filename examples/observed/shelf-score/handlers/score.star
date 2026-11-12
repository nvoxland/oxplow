# shelf-score.score { uri, score }: write the score into this
# extension's own namespace of the book's fields.

def transform(x):
    i = x["input"]
    scope("model.fields", {
        "model": "shelf/book",
        "uri": i["uri"],
        "values": {"score": i["score"]},
    })
    return {"commands": [], "events": [], "result": {"uri": i["uri"], "score": i["score"]}}
