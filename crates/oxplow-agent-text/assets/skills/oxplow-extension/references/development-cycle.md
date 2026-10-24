# The development cycle

An extension has a **source** (the files you edit, in
`.oxplow/extensions/<name>/` or where `extensionSources` points) and
**deployments** (immutable copies oxplow runs, in
`.oxplow/runtime/extensions/<name>/<hash>/`). Nothing runs a source. Your
edits change nothing on the person's screen until you deploy.

## Edit, check, deploy

1. Edit the source. Write as many files as the step needs.
2. Check it after every edit: `oxplow_validate_extension` (the same report
   as `oxplow extension check`), `oxplow_run_lens`, `oxplow_preview_extension`,
   `oxplow_show_lens` with a `spec`. These read the source and dry-run, so
   they need no deploy.
3. When a step is complete, run `oxplow.extension.deploy { name }` with
   `run_command`. Pass `stream` when you work in a worktree stream (the
   source is read from that stream's checkout). The deploy checks the
   source, runs its `build:`, copies it into the store, migrates its
   settings, and replaces what runs in the running app. It returns once the
   new deployment runs, with `warnings` worth fixing. Pages that show the
   extension reload by themselves.
4. Tell the person what changed and where to look.

Deploy at points where the work is whole, not after every edit. Each
deploy is what the person sees, so a deploy in the middle of a change shows
them a half-done page. The same source deployed again is `already: true`
and changes nothing.

## What never needs a restart

An extension change never needs oxplow restarted. Never ask the person to
restart for one: deploy instead. A restart does not deploy anything
either. Boot runs only the deployments that were recorded, so a source you
left half-edited never goes live by itself.

## When a deploy fails

- **It doesn't load or prepare:** the deploy fails with the problems and
  the previous deployment keeps running. Fix the source and deploy again.
- **It asks for approval:** a deploy that adds `permissions:` or a new or
  changed `build:` command becomes a proposal. The person approves it;
  tell them what it asks for and why.
- **A deployment that ran before is now held** (oxplow changed and it no
  longer activates): it stays the chosen deployment and every activation
  tries it again. `v_extension.errors` says why it doesn't run. Fix the
  source and deploy, or go back with `oxplow.extension.rollback { name,
  to? }` (`v_extension_deployment` lists the hashes; the store keeps the
  active one and the five newest).

## A new checkout, a teammate's extension

An extension that is on but was never deployed on this machine is listed
`undeployed` (`v_extension.deployment`), and `v_extension.pending` says
what its deploy will ask for. It runs nothing until someone deploys it.
Deploy it when the person asks, after reading its `permissions:`.

## When oxplow-foundation isn't running

`oxplow-foundation` is oxplow's own required extension: the agent
harnesses, Alerts, Settings and core's commands. If it isn't running, the
app shows a dialog that says so, and `oxplow.extension.deploy` is
unavailable (it is foundation's command). This happens when someone
develops oxplow itself: a core change that foundation's recorded
deployment doesn't activate against. The fix is outside oxplow: quit it,
run `cargo run -q -p oxplow-dev -- extension deploy` in the oxplow
repository, then start oxplow again.
