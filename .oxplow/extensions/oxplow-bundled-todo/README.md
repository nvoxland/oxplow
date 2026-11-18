# oxplow-bundled-todo

Oxplow's own contributions that have not been reviewed yet. This
extension is off (`oxplow.extensions.oxplow-bundled-todo.enabled: false` in
`.oxplow/project.yaml`), it does not ship, and nothing may depend on it.
Each piece moves to `extensions/oxplow-bundled` when it is cleaned up and
has fixtures. When nothing is left, this extension goes away. Do not add
new work here. `.context/extensions-bundled.md` "The `oxplow-bundled`
extension" has the rules for a move.

## What is still here

- **Request review**, mounted on the Changes, commit and uncommitted pages:
  Change Analysis (`change-review`), Tests Weakened, and Usually Changes
  With (`file-co-change`, on a diff's file header).
- **Pages**: Agent Interface and Command Errors, Extension runs and
  Extension Run Detail, Job Runs and Job Run Output, Merge Readiness,
  Remote Branches and Hand Off Work.
- **Custom components**: Explore Data, Timeline and the history graph
  (replaces Git History's plain list at `vcs.history.graph`).
- **Thread parts** with no mount: Plan, Handoffs to and from this thread.
- **Advisories**: `hint-never-fires`, and `shared-files` /
  `shared-files-agent` (two threads changed the same file recently).
- **Commands**: `oxplow.advisory_policy.read` / `.write`, and
  `oxplow.handoff.offer` (the Hand Off Work… menu entry).

`desktop-tests/` and `e2e-specs/` are the old frontend and browser tests of
these pages. They are reference for a move, not specs to restore.

## Settings

The advisories' constants are this extension's `configuration:`. A project
changes them without a fork:

| Key | Default | What it controls |
|---|---|---|
| `sharedFiles.windowMinutes` | 60 | How recent an edit must be for two threads' requests that changed the same file to count as overlapping work |

`oxplow.advisory_policy.read` returns the effective values, where each one
came from, and the stamps that a write needs. `oxplow.advisory_policy.write`
sets the project's or the person's layer, as a compare-and-swap on those
stamps:

```json
{ "schema_version": 1,
  "project": { "expected": { "schema_version": 1, "revision": 2 },
               "values": { "sharedFiles": { "windowMinutes": 30 } } },
  "personal": null }
```

A change takes effect without a restart: the advisory reads it on its next
run.
