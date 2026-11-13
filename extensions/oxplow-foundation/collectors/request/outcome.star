# A request's outcome, recorded each time one of its turns ends while none
# of its turns runs (a request has no finished moment: a later turn records
# it again): the tokens it spent, its cycle time, and how its tests went —
# red at the end, how red it got, which tests broke, how many runs went red,
# and how long it stayed red before the first green.
#
# Gets {"rows": [...]}: the request's own row first ({request, tokens,
# cycle_ms, placed_at}, run NULL; none while a turn of it runs), then one
# row per (run, failed case) of the test runs its turns made, oldest first,
# a run with no failed case on a row of its own: {run, failed, cases, at_ms,
# case}. `case` is the failed test's subject; `cases` how many cases the
# run's report listed (0 for a run that only asserted counts, whose failures
# have no identity). Returns {"facts": [...], "capture": {at, replaces}}:
# the capture is placed at `placed_at` and replaces the request's earlier
# one, so the request counts once, in the period it is placed in.

def transform(x):
    request = None
    tokens = None
    cycle = None
    placed_at = None
    runs = []
    by_run = {}
    for r in x["rows"]:
        if r["run"] == None:
            request = r["request"]
            tokens = r["tokens"]
            cycle = r["cycle_ms"]
            placed_at = r["placed_at"]
            continue
        if r["run"] not in by_run:
            by_run[r["run"]] = {"at": r["at_ms"], "failed": r["failed"], "reported": r["cases"] > 0, "refs": []}
            runs.append(r["run"])
        if r["case"]:
            by_run[r["run"]]["refs"].append(r["case"])
    if request == None:
        return {"facts": []}
    return {"facts": outcome(request, tokens, cycle, runs, by_run), "capture": {"at": placed_at, "replaces": request}}

def outcome(request, tokens, cycle, runs, by_run):
    facts = []
    if tokens:
        facts.append({"measure": "oxplow.request_tokens", "value": tokens, "subject": request, "num": tokens, "den": 1})
    if cycle != None:
        if cycle < 0:
            cycle = 0
        facts.append({"measure": "oxplow.request_cycle_time", "value": cycle, "subject": request, "num": cycle, "den": 1})
    if not runs:
        return facts

    # How many tests failed in each run: the failed cases it listed, or — a
    # run that reported no cases — the failures it asserted.
    failures = []
    anonymous = False
    known = {}
    for id in runs:
        run = by_run[id]
        count = len(run["refs"])
        if count == 0 and not run["reported"]:
            count = run["failed"]
            if count > 0:
                anonymous = True
        for ref in run["refs"]:
            known[ref] = True
        failures.append(count)

    at_close = failures[-1]
    peak = max(failures)
    red_runs = len([n for n in failures if n > 0])
    # Failures with no identity can't be deduplicated: never report fewer
    # distinct tests than the worst single run.
    distinct = len(known)
    if anonymous and peak > distinct:
        distinct = peak
    for stat, value in [("at_close", at_close), ("peak", peak), ("distinct_failed", distinct), ("red_runs", red_runs)]:
        facts.append({
            "measure": "oxplow.request_test_outcome",
            "value": value,
            "subject": request,
            "num": value,
            "den": 1,
            "dims": {"oxplow.tests_stat": stat},
        })

    # From the first red run to the first green one after it; none when it
    # never went red or never recovered (no data, not a zero).
    first_red = None
    for i in range(len(runs)):
        if failures[i] > 0:
            first_red = i
            break
    if first_red != None:
        for i in range(first_red + 1, len(runs)):
            if failures[i] == 0:
                ms = by_run[runs[i]]["at"] - by_run[runs[first_red]]["at"]
                if ms < 0:
                    ms = 0
                facts.append({
                    "measure": "oxplow.request_time_to_green",
                    "value": ms,
                    "subject": request,
                    "num": ms,
                    "den": 1,
                })
                break
    return facts
