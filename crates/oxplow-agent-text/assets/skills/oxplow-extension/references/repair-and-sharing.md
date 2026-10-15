# Health, repair, sharing and installing

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

## Sharing and installing

- **Team:** extensions are ordinary committed files under
  `oxplow/extensions/`. Commit them and the team has them.
- **World:** to publish, put an extension in its own git repo with
  `extension.yaml` at the root. To use someone else's, **only when the
  user asks**: call `review_extension(git_url, git_ref?, stream_id)`, show
  the user what it declares (the programs its collectors run, the hosts they
  reach, the credentials they read, advisories) and any `errors` /
  `problems`, and when they say go, run `oxplow.extension.install { git_url,
  git_ref?, reviewed_sha: <its sha> }` (`run_command`) — it
  comes back as a proposal the user approves. It records the source in
  `source.yaml`. Updating is the same: `review_extension(name)`, then
  `oxplow.extension.update { name, reviewed_sha }`. Never
  hand-edit an installed extension's files; they're overwritten on update.
  Copy it into a new extension instead.
