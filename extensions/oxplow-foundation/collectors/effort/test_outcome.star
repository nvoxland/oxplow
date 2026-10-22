# An effort's test outcome, recorded when it finishes: did it close green, how
# red did it get, which tests broke, how many runs went red, and how long it
# stayed red before the first green.
#
# Gets {"rows": [...]} — one row per (run, failed case) of the effort's
# measured test runs, oldest first, a run with no failed case on a row of its
# own: {run, failed, cases, at_ms, case, effort}
# (`effort` the bare effort id, `eff3`; the facts' subject is its URI). `case` is the failed test's
# subject (`oxplow:oxplow/test?class=<classname>&name=<name>`); `cases` how many cases the run's report
# listed (0 for a run that only asserted counts, whose `failed` failures have
# no identity); `at_ms` when it was recorded. Returns {"facts": [...]}.
# No runs is no facts.

def transform(x):
    runs = []
    by_run = {}
    effort = None
    for r in x["rows"]:
        effort = uri.build("oxplow/effort", id = r["effort"])
        if r["run"] not in by_run:
            by_run[r["run"]] = {"at": r["at_ms"], "failed": r["failed"], "reported": r["cases"] > 0, "refs": []}
            runs.append(r["run"])
        if r["case"]:
            by_run[r["run"]]["refs"].append(r["case"])
    if not runs:
        return {"facts": []}

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

    facts = []
    for stat, value in [("at_close", at_close), ("peak", peak), ("distinct_failed", distinct), ("red_runs", red_runs)]:
        facts.append({
            "measure": "oxplow.effort_test_outcome",
            "value": value,
            "subject": effort,
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
                    "measure": "oxplow.effort_time_to_green",
                    "value": ms,
                    "subject": effort,
                    "num": ms,
                    "den": 1,
                })
                break
    return {"facts": facts}
