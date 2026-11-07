# Sourced by the test runs: tests make their temp folders in one of their own.
#
# The machine's temp folder holds whatever every other project left there
# (236,000 entries on one machine, 12 s to list), and tests leak into it too: a
# background task can write a project's `.oxplow` again after its test removed
# the folder. Worktrees share this folder, so what goes is only what is a day
# old: a run in another worktree may be using the rest. The name is short because
# a unix socket's path has to fit in about 100 bytes.
TMPDIR="${TMPDIR:-/tmp}"
TMPDIR="${TMPDIR%/}/ox"
export TMPDIR
mkdir -p "$TMPDIR"
find "$TMPDIR" -mindepth 1 -maxdepth 1 -mtime +0 -exec rm -rf {} + 2>/dev/null || true
