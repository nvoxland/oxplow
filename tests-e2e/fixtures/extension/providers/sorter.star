# A scripted model provider with no model: it answers each choice question
# with "bug" when the text it was asked about mentions a bug (and "bug" is
# an option), else "cleanup", else the first option. The host's one call goes
# to the instance's baseUrl; the spec's server there echoes the body back,
# so the answer is decided here, in `response`.

def request(x):
    questions = {}
    for name, q in x["questions"].items():
        questions[name] = q["options"] if q["type"] == "choice" else []
    return {"path": "/sort", "body": {"text": x["state"], "questions": questions}}

def response(x):
    text = x["body"]["text"].lower()
    answers = {}
    for name, options in x["body"]["questions"].items():
        pick = options[0] if options else ""
        if "bug" in text and "bug" in options:
            pick = "bug"
        elif "cleanup" in options:
            pick = "cleanup"
        answers[name] = {"type": "choice", "choice": pick}
    return {"answers": answers, "usage": {"input": 1, "output": 1}}
