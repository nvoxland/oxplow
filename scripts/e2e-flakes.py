"""Summarize the browser suite's JUnit reports, one folder per CI run.

Called by e2e-flakes.sh with the folder that holds them. A case that failed
has a `failure` or `error`; one that passed only on its retry has a
`flakyFailure` or `flakyError` (the JUnit reporter's `includeRetries`)."""
import collections
import os
import sys
import xml.etree.ElementTree as ET

root = sys.argv[1]
broke = collections.Counter()
flaked = collections.Counter()
runs = []
for run in sorted(os.listdir(root)):
    path = os.path.join(root, run, "junit.xml")
    if not os.path.exists(path):
        continue
    sha = open(os.path.join(root, run, "sha")).read().strip()
    suite = ET.parse(path).getroot()
    outcomes = {}
    total = 0.0
    # A project's cases are a testsuite of their own, named by `hostname`.
    for project in suite.iter("testsuite"):
        for case in project.iter("testcase"):
            key = f"[{project.get('hostname')}] {case.get('classname')} :: {case.get('name')}"
            if case.find("failure") is not None or case.find("error") is not None:
                broke[key] += 1
            elif case.find("flakyFailure") is not None or case.find("flakyError") is not None:
                flaked[key] += 1
            outcomes[key] = True
            total += float(case.get("time") or 0)
    wall = float(suite.get("time") or 0)
    runs.append((run, sha, len(outcomes), wall, total))

print(f"{len(runs)} runs read\n")
print("failed  flaked  spec")
for key in sorted(set(broke) | set(flaked), key=lambda k: -(broke[k] + flaked[k])):
    print(f"{broke[key]:6}  {flaked[key]:6}  {key}")
print("\nrun          sha        tests   wall s   sum s  busy workers")
for run, sha, n, wall, total in runs:
    busy = total / wall if wall else 0
    print(f"{run:12} {sha:9} {n:6} {wall:8.0f} {total:7.0f}  {busy:5.2f}")
