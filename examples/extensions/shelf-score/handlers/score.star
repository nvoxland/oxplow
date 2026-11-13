# shelf_score.book.score { uri, score }: write the score into this
# extension's own namespace of the book's fields.

def transform(x):
    i = x["input"]
    return {
        "commands": [{"name": "oxplow.model.set_fields", "input": {
            "model": "shelf/book",
            "uri": i["uri"],
            "values": {"score": i["score"]},
        }}],
        "events": [],
        "result": {"uri": i["uri"], "score": i["score"]},
    }
