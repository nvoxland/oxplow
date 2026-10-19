# Health, repair, deploying and sharing

## Health and repair

oxplow keeps each provider's, collector's and effect's health on this machine
in `v_contribution_health` (`extension`, `contribution`, `kind`, `state` = `ok` |
`failing` | `disabled`, `reason`, `last_error`, `dead_letters`, `fresh`,
`repair_item`). Each collector's last run is in `v_collector_run`
(`status`, `last_run_at`, `error`).

- **Three failures in a row disable it.** A disabled collector doesn't
  run, `oxplow.collector.sync` refuses naming the reason, and a lens over its
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
  returns once it runs. If the extension does not prepare, the deploy fails
  with the problems and the old deployment keeps running.
  `oxplow.extension.rollback { name, to? }` makes an earlier deployment live.
- **Team:** a new extension's source is personal. `.oxplow/.gitignore` keeps
  it out of git. `oxplow.extension.share { name }` un-ignores it and moves
  the name to the team list in `.oxplow/project.yaml`; the person then
  commits. `oxplow.extension.unshare { name }` reverses it.
- **Use someone else's extension:** only when the user asks. Clone it into
  `.oxplow/extensions/<name>/` (or point `extensionSources` at the clone),
  list it in `extensions:` or `enableExtensions`, and deploy it. Read what
  it declares first (the programs its collectors run, the hosts they reach,
  the credentials they read). Do not hand-edit a clone you want to update
  later; copy it into a new extension instead.
