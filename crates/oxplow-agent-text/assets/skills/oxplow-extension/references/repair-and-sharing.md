# Health, repair, deploying and sharing

## Health and repair

oxplow keeps each provider's, collector's and effect's health on this machine
in `v_contribution_health` (`extension`, `contribution`, `kind`, `state` = `ok` |
`failing` | `disabled`, `reason`, `last_error`, `dead_letters`, `fresh`,
`repair_item`). Each collector's last run is in `v_collector_run`
(`status`, `last_run_at`, `error`).

- **Three failures in a row disable it.** A disabled collector doesn't
  run, `oxplow.collector.sync` refuses naming the reason, and a page over its
  view carries the reason as a warning.
- **A disable files a repair work item** (`repair_item`). Its body is the
  whole repair brief: what the extension is for, the declaration, the
  last errors, the failing fixture and the `check` report. A repeat
  failure comments on the open item. The item is linked `about` the
  contribution's ref (`v_work_item_link`), which is how the open one is
  found; its title is only for people. When the user asks you to repair an
  extension, read that item first.
- **Fix it, then prove it.** Change the script or declaration, run
  `oxplow extension check` and `oxplow extension test`, then `oxplow.collector.sync`
  (`run_command`) — a refusal while it's disabled is expected.
- **You can't enable it again.** `oxplow.contribution.enable` (and `enable_all`) is the user's: tell them
  to press **Enable Again** (or **Enable All Disabled**) in Settings → Extensions once your fix is in.
- **Undelivered events** (a consumer that failed on an event) are in
  `v_event_dead_letter` (`state = 'pending'`); the user retries or
  discards them in Settings → Data → Delivery.

## Deploying and sharing

- **Deploy:** nothing runs a source. After an edit is right, run
  `oxplow.extension.deploy { name }` (`run_command`). It copies the source
  into `.oxplow/runtime/extensions/<name>/<hash>/`, makes that copy live and
  returns once it runs, replacing what ran: no restart. If the extension
  does not prepare, the deploy fails with the problems and the old
  deployment keeps running. Boot deploys nothing, so a restart never
  publishes a source (`references/development-cycle.md`).
  `oxplow.extension.rollback { name, to? }` makes an earlier deployment live.
- **Needs a tool to build** (TypeScript, a bundler): declare
  `build: { run: <command>, output: dist, exclude: [node_modules] }`.
  The deploy runs `run` in the source folder, then deploys the inputs plus
  `output`; editing `output` by hand changes nothing. A new or changed
  `run` is approved by the person like a permission, so your deploy then
  becomes a proposal. Point a component's `bundle:` into `output`
  (`dist/<id>`); validate accepts it before the first build.
- **Needs another extension on** without importing from it: `requires:
  [<name>]`. Deploying refuses while it is off.
- **Team:** a new extension's source is personal. `.oxplow/.gitignore` keeps
  it out of git. `oxplow.extension.share { name }` un-ignores it and moves
  the name to the team list in `.oxplow/project.yaml`; the person then
  commits. `oxplow.extension.unshare { name }` reverses it.
- **Use someone else's extension:** only when the user asks. Clone it into
  `.oxplow/extensions/<name>/` (or point `extensionSources` at the clone),
  list it in `extensions:` or `enableExtensions`, and deploy it. Read its
  `permissions:` first (the programs it runs, the hosts it reaches, the
  secrets it reads): the deploy asks the person to approve them.
  Do not hand-edit a clone you want to update
  later; copy it into a new extension instead.

`v_extension_source` is each source as it is now, with the hash a deploy of
it would give: one unlike `v_extension.hash` has changed since it was
deployed.
