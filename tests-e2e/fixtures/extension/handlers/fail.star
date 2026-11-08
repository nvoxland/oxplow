# e2e.fail.now: always refuses, so the suite can see how a person's failed
# command shows (beside the control that ran it, recorded once).

def transform(x):
    return {"refuse": "the suite asked for a failure"}
