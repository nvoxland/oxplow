# The development cycle

An extension has a **source** (the files you edit, in
`.oxplow/extensions/<name>/` or where `extensionSources` points) and
**deployments** (immutable copies oxplow runs, in
`.oxplow/runtime/extensions/<name>/<hash>/`). Nothing runs a source. Your
edits change nothing on the person's screen until you deploy.

## Fixture, edit, check, test, deploy

1. Write the fixture for what you're about to build
   (`references/testing.md`) and run `$OXPLOW_BIN extension test <name>`:
   it fails, for the reason you expect.
2. Edit the source. Write as many files as the step needs.
3. Check it after every edit: `oxplow_validate_extension` (the same report
   as `$OXPLOW_BIN extension check`), then run its fixtures with
   `$OXPLOW_BIN extension test <name>` in your terminal until they pass.
   Both run on a throwaway oxplow — this build's schema, only the data a
   fixture makes, only the extensions yours declares (a fixture adds more with `requires:`) — so they need no
   deploy and nothing in the project's database affects them.
   `oxplow_run_page` (`draft: true` in a worktree) shows a page over the
   project's real data: a look before you deploy, not a test.
4. When a step is complete, run `oxplow.extension.deploy { name }` with
   `run_command`. It deploys the source in the main stream's checkout,
   uncommitted changes included: deploy to let the person see a change,
   and commit once they are happy with it. In another (worktree) stream,
   your copy of an extension that lives in the repo is a draft: check and
   test it there; it is refused for deploy and goes live once your change
   reaches the main stream's branch. `oxplow.extension.draft_status { name }`
   says how your draft differs from what runs (pages and commands added,
   removed, changed). The deploy checks the
   source, runs its `build:`, copies it into the store, migrates its
   settings, and replaces what runs in the running app. It returns once the
   new deployment runs, with `warnings` worth fixing. Pages that show the
   extension reload by themselves.
5. Tell the person what changed and where to look.

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
deployment doesn't activate against. The dialog says why it is held. The
fix is outside oxplow: tell the person to reinstall oxplow (or update it),
then start it again.

## What already exists

Prefer improving a page over adding a near-duplicate. `oxplow_find_page`
searches by question; in SQL `v_page` (`uri`, `extension`, `title`,
`category`, `hidden`, …) lists every page, `v_page_answer` the questions
they answer and `v_page_param` their params; `v_extension` lists the
extensions that loaded (`enabled`, `errors`, `warnings`, `deployment`) and
`v_contribution` what each contributes (count it by `extension` and `kind`).
`oxplow_read { uri: "oxplow:oxplow/page?page=<ext>/<slug>" }` gives one
page's definition and deployment; **Definition** in a page's ☰ menu, About this page, is
the same for the person (the running page file, drawn as Definition, a
display mode of the file beside the File viewer's raw YAML). An extension that fails to load isn't in
`v_extension`: `oxplow_validate_extension` says why.

## When the person says "this page"

When the user says "this page", "what I'm looking at" or pastes a page
reference: call `oxplow_get_open_page` (its tree is what they see, with
their params and focus; `#node=<id>` names a part), read the definition with
`oxplow_read`, change the file, then validate and run it again
(`oxplow_run_page { draft: true }`). Tell them the title and how to open it
(Cmd+P), and offer to commit the source if they want their team to have it.
