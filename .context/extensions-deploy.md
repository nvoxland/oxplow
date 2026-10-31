# Extensions: Sources, deployments and sharing

Sources and deployments, builds, sharing, disabling and reviewing a change by impact. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Sharing, settings checks and disabling

- **Sharing.** There are three levels:
  - **Yourself.** A source is personal (`.oxplow/.gitignore` ignores it). It
    runs after `oxplow.extension.deploy`.
  - **Your team.** `oxplow.extension.share { name }` un-ignores the source;
    the person commits it. A teammate deploys it in their checkout (Settings
    → Extensions lists it `undeployed` with Deploy): boot deploys nothing.
  - **Someone else's.** The interim way: clone it into
    `.oxplow/extensions/<name>/` (or point `extensionSources` at it), list
    it, deploy it.
  - A teammate's extension runs nothing until a person deploys it here.
    An exec source, a Starlark one that calls a model (`ai_*`) and
    advisories need `permissions:` the manifest declares, and a deploy
    that adds a permission asks a person first (extensions-permissions.md "Permissions"); a
    deployment is immutable, so what runs is what was deployed (see
    [architecture.md](./architecture.md) → "A repo's config never runs a
    program without consent").

- **Settings shows what `extension check` finds.** Loading can't
  catch a page whose SQL fails (it never runs the SQL), so Settings →
  Extensions runs `oxplow_validate_extension` — the same dry run `oxplow extension
  check` prints — for each enabled project extension, on open and on
  `extensionsChanged`, and lists its errors with the load errors, once
  each. Bundled extensions are checked where they're built.

- **Disabling.** An extension that is off contributes nothing: it has
  `enabled: false` and no pages, slots or sources, so every consumer
  ignores it; `find_page` and `run_page` say it's disabled. Which are
  on is the team's `extensions:` plus the person's `enableExtensions` /
  `disableExtensions` ("Sources and deployments"). The Extensions page
  toggles it through `oxplow.extension.enable` / `disable`.

## Sources and deployments

An extension has a **source**, the files a person or their agent edits,
and **deployments**, the immutable copies oxplow runs. Nothing runs a
source directly: an edit changes nothing until it is deployed.

- **Source:** `.oxplow/extensions/<name>/` in a checkout, unless
  `extensionSources: { <name>: <folder> }` points the name elsewhere. The
  folder is relative to the checkout or absolute (`~/` is the home folder).
  The team's `extensionSources` is in `.oxplow/project.yaml`; a person's own,
  which wins, is in `.oxplow/personal.yaml`. This repo points
  `oxplow-foundation` and `oxplow-bundled` at `extensions/…`, so their
  sources are ordinary repo files. A personal source outside every checkout
  (`~/oxplow-extensions/x`) is how an agent in any stream edits it: the write
  guard only blocks paths inside another stream's checkout.
- **One deployable source.** The settings (which names are on, their
  `extensionSources`) are always the primary checkout's. A source inside the
  project is deployed only from the **primary** stream's checkout (the one
  the project was opened from, whichever branch it is on), uncommitted
  changes included. Another stream's checkout holds a **draft** of it: that
  stream's agent edits, checks and tests it there, and it deploys once the
  change reaches the primary's branch. A source outside the project has one
  copy, so any stream deploys it (`extension_deploy::refuse_draft_deploy`).
  `oxplow.extension.draft_status { name, stream_id? }` (a read,
  `extension_deploy::source_diff`) says how a stream's source differs from
  the running version: pages and commands added,
  removed and changed, and whether the checkout is a draft. A deploy that
  removes a page or command still goes, with a warning naming them.
- **Deployment:** `.oxplow/runtime/extensions/<name>/<hash>/`, where
  `<hash>` is the hash of the source (`extension_store::source_hash`: sha256
  over each file's relative path and bytes, dot entries skipped, cut to 16
  hex characters). The hash is the extension's identity; a deployment
  folder is written whole (copied beside, then renamed) and never changed.
  A source with a `build:` hashes its **inputs** — every file but
  `build.output` and the `build.exclude` folders — plus its `build.run`
  text (`extension_store::Inputs`; a manifest without `build` hashes as it
  always did), and its deployment holds those inputs and the built output,
  never the excluded folders ("Builds").
  Everything oxplow reads for a deployed extension — manifest, pages,
  Starlark, datasets, component bundles, provider entries, skills — comes
  from that folder. Its `Extension.path` is the folder's absolute path, so
  every `root.join(ext.path)` reader finds it. The file watcher never looks
  under `.oxplow/runtime/`; it does watch `.oxplow/extensions/` (an agent's
  edits to a source are its work, snapshotted and given to its effort), and
  watching never activates anything.
- **Shipped deployments:** `oxplow-foundation` and `oxplow-bundled` ship
  inside oxplow (`bundled_extensions::BUNDLED`, each one's files through
  `BundledExtension::files()`) and are written to the store once per hash as
  their shipped deployment (`extension_store::extract_shipped`); they run
  from disk like any other. What ships is the folder: every file but dot
  entries and `build.exclude`, its build's output included.
  - **A release build compiles them in.** `crates/oxplow-app/build.rs`
    walks each folder into `include_bytes!` arrays, so adding a file to the
    folder ships it; one with a `build:` is built there first (its command,
    `/bin/sh -c` in its folder), so its output ships without being
    committed — building oxplow needs the tools those builds use (bun).
    Cargo is told to rerun on the manifest, every input file, and the input
    folders that hold nothing skipped — never a folder holding the output,
    `node_modules` or a dot entry, which cargo would check recursively and
    find rewritten by the build every time. So a new top-level file (or one
    in a folder like that) is noticed only after `extension.yaml` is
    touched.
  - **A debug build (and every Rust test) reads them from their folders**
    the first time they're asked for, once per process
    (`extension_store::shipped_files`), so editing an extension never
    recompiles `oxplow-app` or anything above it. Its `build.rs` watches only
    itself and runs a `build:` only when its `output` folder is missing (a
    fresh checkout); a changed source reaches a running dev oxplow through
    deploy, which builds. A test reads the folder as it is on disk, edits
    included, the moment it runs.
  - The shipped hash uses the source rule
  (`extension_store::shipped_hash`: the inputs and the build command,
  never the output), so deploying an unchanged
  `extensions/oxplow-foundation` resolves to the shipped deployment. A
  bundled extension with no active deployment of its own runs the shipped
  one (`Deployment::Shipped`).
  - **A folder never stands in for one.** Outside development a source
    load (`load_sources`, `source_or_shipped`) takes a shipped name's copy
    built in, never `.oxplow/extensions/<name>/`: a project folder with
    that name is ignored with a warning on the shipped extension, so a
    cloned repo can't load its own `oxplow-bundled` as oxplow's. A load
    therefore holds one entry per name, and `extensions::owner(list,
    name)` is the one lookup by name — none tie-breaks on origin
    (`source_guards::no_lookup_by_name_prefers_an_origin`).
- **Developing oxplow (`enableBundledExtensions: false`):** in a dev
  build (`cfg!(debug_assertions)`; a release build logs and ignores it),
  oxplow's own extensions don't run from the copies built in: they load
  from their `extensionSources` folders and are deployed like any
  extension (`bundled_extensions::shipped(name, &settings)` is the one
  accessor for the shipped files; `find`, `is_required`, `is_reserved`
  and `origin_of` stay name-based, so the `oxplow` namespace and the other
  rules for oxplow's own extensions still apply). Set in `project.yaml`
  (this repo commits it), overridable in `personal.yaml`, read
  once when the catalog is made (`ExtensionCatalog::settings`): changing
  it takes a restart. Boot deploys nothing here either: a checkout's first
  run, and a core change that foundation's recorded deployment doesn't
  activate against, need `cargo run -q -p oxplow-dev -- extension deploy`
  with oxplow quit (`extension_deploy::DEPLOY_FROM_OUTSIDE`; it deploys
  every extension that is on whose source isn't what runs, foundation
  first). Nothing ships to roll back to, and a non-required one can be
  removed.
- **The live set:** each extension's active deployment, in the
  `extension_deployment` table (`v_extension_deployment`: name, hash,
  deployed_at, commit, dirty, source, active). `commit` is the primary
  checkout's HEAD when the source lies inside the project (NULL outside it)
  and `dirty` says the source had uncommitted changes, untracked files
  included: an uncommitted deploy is normal, and rollback names the
  target's commit. The publish transaction of every
  activation marks the active rows from what it published, so the table
  and what runs never disagree. The store keeps the active deployment and
  the five newest (`extension_store::KEEP`); deploy prunes the rest.

**Which extensions are on** is two lists: the team's `extensions: [..]` in
`.oxplow/project.yaml` (absent: `[oxplow-bundled]`; `[]` turns it off) and
the person's `enableExtensions` / `disableExtensions` in
`.oxplow/personal.yaml`. On = (team ∪ enable) − disable
(`oxplow_config::ExtensionSettings::enabled`). `oxplow-foundation` is always
on and never listed. A listed name with neither a source nor a deployment
on this machine is listed as `not_here` (`v_extension.deployment`), not an
error, so a teammate's personal extension in the team list breaks nothing.

**Two ways to load** (`extension_catalog.rs`):

- **Deployed** (the daemon's catalog, `ExtensionCatalog::for_project`): the
  live set, the same whatever root a caller names. An extension that is on
  but has no deployment is listed `undeployed`, not run.
  `extensions::load_deployed`.
- **Source** (standalone `ExtensionCatalog::new()`, the CLI, the SDK, the
  authoring checks `oxplow_validate_extension` / preview): each source
  folder in a checkout, every one present on unless `disableExtensions`
  lists it (`extensions::sources_on`) — what the author's sources would do.
  `extensions::load_extensions`.

**Deploy** (`oxplow.extension.deploy { name }`, the agent's
`run_command`; the CLI has no way to reach the running daemon yet, so it
has no deploy; `extension_deploy::deploy`): a caller in a stream other than
the primary is refused when the source lies inside the project
(`refuse_draft_deploy`, naming the check and test tools and the branch it
deploys from) → the source in the primary checkout → hash → already active is `already: true` →
source load, whose errors refuse it → copy into the store and record the
row → turn it on in the person's layer when nothing turns it on → stage the
live set with it in (`ExtensionCatalog::stage_changes`, which every staging
check's `snapshot()` then reads) and publish. An extension the staging holds
back (it doesn't prepare) keeps its previous deployment and the deploy
fails with its problems; a candidate overtaken by another change is staged
again. The command returns once the deployment runs; restart starts from
the recorded live set. Nothing caps an
extension's size: a deployment over 25 MiB goes live with a warning in the
result's `warnings` (every deployment copies the whole folder).
`oxplow.extension.rollback { name, to? }` makes an earlier deployment
active (`to` a hash or its start; a bundled one with nothing older returns
to the shipped deployment). `oxplow.extension.enable` / `disable` write the
person's lists (enable deploys one with no deployment here).
`oxplow.extension.remove` takes it out of both lists, stops it, forgets its
deployments, private state and settings, and never deletes its source.

**Builds.** `build: { run, output, exclude? }` is for a component or a
program that needs a tool to make (TypeScript, a bundler). Deploy runs
`run` with `/bin/sh -c` in the source folder (`extension_build::run`:
this process's environment with the person's `PATH` and `HOME` from the
host env, a process group, a 10-minute timeout, the end of its output in
a failure), then copies the inputs and `output` into the deployment and
loads the deployment folder: one that doesn't load once built (a bundle
the build didn't write) is refused (``extension `x` doesn't load once
built``) and leaves nothing behind. A build that fails, or doesn't write
`output`, refuses the deploy with ``building `x`: …``. A build runs with
the person's authority and oxplow doesn't confine it, so its command is
approved like a permission: a new or changed `run` adds ``build — runs
`…` in <source> with your authority, on every deploy`` to what the deploy
asks (`extension_deploy::build_line`); the same command never asks again.
The approval covers the command text only — what `npm run build` does is
whatever `package.json` says. The recorded command
(`extension_deployment.build_run`, `v_extension_deployment.build`) is what
rollback compares; rollback never rebuilds, and neither does a deploy whose
hash is already in the store. A component bundle under `output` loads from
the source as declared (`BundleLook::Unknown`) while `output` doesn't
exist yet, so validate and preview pass before any build. A bundled
extension's build runs when oxplow is built ("Shipped deployments"); while
developing oxplow its deploy (from `oxplow-dev` or the app) runs the build
like any extension's. `oxplow extension check`
runs no build (the built source isn't deployed in its throwaway);
`extension test` builds, as the author's own run; its copy links the
`exclude` folders instead of copying them.

**Startup** (`extension_deploy::start`, before the first activation)
**deploys nothing.** The live set starts from the active rows whose folder
is still in the store, and that is all boot runs. An extension that is on
with no deployment here (a fresh clone, a just-pulled teammate's) is listed
`undeployed`, its `pending` naming what its deploy will ask for, until a
person deploys it. So what runs is only ever what someone deployed, never a
source caught half-edited. (Until 2026-10-24 boot deployed those sources
itself; with a held deployment losing its record, that made a core change
take two restarts and put whatever the source held live unasked.)
Boot is one activation (`ExtensionActivation::initialize`: `start`, then
the first generation).

**The live set is what was chosen, not what happened to run.** A
deployment that is held — it doesn't load or doesn't prepare against this
oxplow — keeps its row `active` and its place in the catalog's live map,
whatever runs in its place (its last working version, the copy that
ships, or nothing): every activation and every boot tries it again and
lists why it doesn't run (`v_extension.errors`), until a person deploys
another or rolls back. The publication records the published deployments
plus each held name's chosen hash (`activate_kind`: `chosen`, read before
the change is staged). A change that is held was never chosen, so a failed
deploy still changes nothing. Because the chosen hash of a held extension
isn't running, `deploy`'s "already" check also asks that it runs: deploying
the same source again activates it again.

A deployment of an extension that ships with oxplow that is held with
nothing of it running — it doesn't load or doesn't prepare — **gives way**
to the copy that ships (`activate_kind`: out of the staged changes,
admitted again); its problems say "the shipped version runs until it is
deployed again", and it stays chosen as above. While developing oxplow
nothing ships, so a broken or missing foundation is just not there: the
rest activates and its problem says why. `start` reports a required
extension with no deployment at all; `explain_required` reports one whose
deployment doesn't activate. Both lines say what is missing without it
(the agent harnesses, Alerts, core's commands and pages) and how to bring
it back from outside — quit oxplow, run `cargo run -q -p oxplow-dev --
extension deploy`, start it — since the commands that deploy are
foundation's own. It is logged, listed in `v_extension.errors` by the
activation that finds it (startup's own problems too: `initialize` hands
them to the first activation), and stands across later activations until
it runs. The desktop shell draws one plain dialog for it
(`apps/desktop/src/foundationDown.ts`, `FoundationDownDialog` in
`App.tsx`): the problem lines and that command, Copy and Dismiss; nothing
else in the app handles a missing foundation. The usual cause is a
deployment an older oxplow recorded meeting a newer core: core and
foundation's source change together in one commit, but the recorded
deployment is the source as it was. `oxplow-dev extension status` shows
what the next boot would run and hold back without starting it
(`extension_deploy::next_boot`: the active deployments activated in a
throwaway in memory), and `oxplow-dev extension deploy [<name>]` deploys
with oxplow quit (`.context/working-in-this-repo.md` "oxplow-dev"). Boot
stops only when a generation can't publish at all (the host, the
database).

**Sharing is one explicit act.** `.oxplow/.gitignore` ignores everything
but `project.yaml`, so a new extension's source is personal.
`oxplow.extension.share { name }` appends `!extensions/`, `extensions/*`,
`!extensions/<name>/` and `!extensions/<name>/**` (git can't re-include a
file under an excluded folder, and `*` matches files at every depth) and
moves the name from the person's `enableExtensions` to the team's
`extensions`; the person commits. A built extension's `output` and
`exclude` folders get their own ignore lines (`extensions/<name>/dist/`)
after those: they aren't source. `oxplow.extension.unshare` reverses it,
removing every line under `extensions/<name>/` whatever the manifest says
now.
Only a source in `.oxplow/extensions/` is shared this way.

**Requires.** `requires: [name, …]` names the other extensions this one
needs on (`extension_deploy::requirements_met` / `not_required`). Deploying
or enabling it refuses while one is off (``"`acme` requires `tracker`,
which is off (turn it on first)"``); disabling or removing one refuses
while an enabled extension requires it. Activation holds an enabled
extension whose requirement isn't among the admitted ones, with the same
last-working-version rule as an unavailable import
(`CatalogSnapshot::admitted`: "it requires `x`, which isn't active"). An
entry must be a valid extension name other than its own, and a cycle is a
load error on each member (`refuse_requires_cycles`). A required bundled
extension (foundation) always counts as on. Unlike `imports:`, `requires`
names no contract: it is for composing another extension's commands or
reading its datasets without an export. **Using another extension without
declaring it is an error** (`check_dependencies`, in `validate_extension`):
reading its dataset (`v_<namespace>_…` in a page, a dataset or a script),
naming its page (`<name>/…`), mounting at its slot or running its command
(`<namespace>.…`, unless the required foundation shares the namespace) —
one finding per extension, at the first place, `fix: requires: [x]`. The
required foundation needs no declaration. A test turns on only what is
declared, so the check says why before a fixture fails on a missing table.
A metric's or a collector's measure must be declared by the extension,
foundation or one it `requires:` (`check_measures`, also in
`validate_extension`, where the other extensions are loaded); any other is
a warning, so a typo isn't silent.

**Writing a page** (`oxplow.extension.write_page { extension, page, spec,
deploy? }`, `extension_deploy::write_page`) writes `pages/<page>.yaml` into
the extension's source (created with a scaffolded manifest when missing)
in the primary checkout, whichever stream asks. The spec must parse as a
page and the extension must still load with it, or the file is put back
and the call fails with the loader's reason. With `deploy: true` it then
deploys, like any deploy (it asks when the source gained a permission).
`oxplow.extension.new_dashboard` writes an empty dashboard page the same
way. A bundled extension is never written to
(`extensions::writable_extension_dir`).

A malformed extension shows an error for that extension only; it never
breaks the others or core.

**Lifecycle scenarios.** `crates/oxplow-app/src/extension/extension_lifecycle_tests.rs`
runs an extension through its life as a person or agent would: every
transition is a bus command (`oxplow.extension.deploy`, `disable`,
`enable`, `rollback`, `oxplow.capability.choose`), every read is what users
see (`v_*` over the SQL gateway, the extension's own commands, page runs).
One test per story: a redeploy that changes a command, a dataset, a
collector and an effect (the old collector's rows stay); disable then enable
(the dataset, page, command and an importing consumer go and come back, a
paced collector run that was due runs after enable); rollback (the earlier
version returns, private state outlives it, a configuration layer written at
a newer schema fails closed until the rolled-back version's command writes
it again at its own schema); a deploy that loads but does not activate (the running
version keeps working and the error says why); private state read by the
next version; a producer's export changing under two consumers (the
unversioned one keeps working; the one pinned to the old contract is held
with a problem naming the import, and keeps serving its last working
version); switching the work list between oxplow's and a provider's; and a
queued job across disable then enable (it waits, then runs). The shared
setup is `test_fixtures::lifecycle` (`booted()` registers the pump
reactors a running oxplow has but in-memory services lack).

## Reviewing by impact

A review looks at an extension's **impact**: what changing it would
change. It does not only look at what the extension declares
(`extension_impact.rs`). `ImpactReport` holds these lists, each entry
with a `Change` (`added`, `removed`, `changed`, `unchanged`):

- `pages` — each page's rendered text
  before and after (`PageImpact`);
- `datasets` — each view, the parts that differ (`changed`: `query`,
  `columns`, `description`, `tests`, `version`, `deprecated`), its
  columns before and after, the first contract difference
  (`contract_change`; a SQL-only change keeps the contract) and its
  `downstream` readers (`datasets_diff`). When the contract held, the
  review names the parts ("its description and tests changed");
- `collectors` — each collector's `Grants` (entry, runtime, hosts,
  credentials, env) before and after and the views it fills
  (`collectors_diff`);
- `providers` — grants, each declared command by name (`CommandChange`)
  and the capability's features before and after, plus where spec and
  declarations first differ (`first_difference`, so a change that no
  grant, command or feature shows still reads as something)
  (`providers_diff`, over each spec and its checked-in declarations);
- `effects` — each effect contribution's trigger and reactions
  (`EffectImpact`, below);
- `contributions` — every other contribution kind, added, removed or
  changed, as one generic row each from the descriptors
  (`ContributionImpact`, `contributions_diff`; `differs` names which of
  `version`, `needs`, `runs_on` and `global` changed).

`extension_impact::json_difference` is the one "where do two JSON
values first differ" walk (the provider host's `first_difference` uses
it too). A provider's changes are worded once, in Rust
(`extension_impact::provider_phrases`): the review's `summary`
prefixes each phrase with `Provider <id>:`, and `approval_lines` — sent
as `ProviderImpact.lines` — capitalizes them for the review's provider
rows.

The report is built from the two loaded versions. **It never runs a
program, an exec collector or an external provider** — a version that is
not yet deployed has no permissions approved. What it does run is
read-only or sandboxed: page and dataset queries on each version's
overlay, and derived collectors' and effects' scripts (below).

`extension_impact::impact(layer, before, after)` builds it. Each
`Version` carries its pages **already dry-run once**
(`pages::checks::dry_run_all` → `PageChecks`): every `sql` loader runs
once with its params at their defaults and no viewer context; `command`
and `page` loaders do not run, because a check acts as no one. For the
candidate these are the very runs `check_extension` made, so a review
runs each page once. Each page is rendered from its dry run
(`pages::checks::text`: each viz node under its title, through
`viz_text::render`) against **its own version's datasets** (each side's
overlay). So a page over a dataset that the candidate changes renders
against the changed SQL. A page that fails the same way on both sides
is `unchanged`. A dataset's `downstream` is its direct readers from
`v_dataset_lineage` (`downstream_of`) other than this extension's own
views, by exact name. A provider's declarations are read from each
version's files. `extension_impact::summary` is the one wording that
`extension check --impact` and an effort's review share: collectors'
and providers' grants first ("now reaches x (was y)", a provider command
added (destructive)), then what the collectors make of their inputs,
then datasets with their readers and rows, then pages ("Page x: changed",
"Page x: its query fails: …"), then the other contributions. A changed
page carries its text before and after in the report (`before`,
`after`); there is no line diff yet.

**An extension at any revision.** `extension_at(trees, ws, rev,
name)` loads project extension `name` as revision `rev` of the
workspace holds it — a commit (`git:HEAD`), a snapshot, the working
tree — through `Trees::corpus` into an in-memory `Tree` (the
`ExtensionFiles` the loader reads). A custom component's bundle is
looked up three ways (`BundleLook`). On disk it is `Found` or `Absent`
(an error). In a revision's tree it is checked from the tree's own paths
when they are there, and `Unknown` — taken as declared, not an error —
when they are not, since a built bundle usually is not committed. At
`git:HEAD` of a clean worktree it equals the disk load. It is `None`
when that revision has no `extension.yaml`. This lets a review compare
two revisions of an extension, neither of which needs to be on disk.

**Each side on its own overlay.** A check is `prepare` →
`Prepared { pages, overlay }`: the extension's datasets (with the other
enabled extensions') compile to temp views, published nowhere, and its
pages, advisories and commands read through that overlay. A review
prepares **both** sides — the candidate and the version before it — so a
page over a dataset whose SQL changed renders against each version's own
SQL, whatever the database has published (a disabled extension, a dataset
that failed, another worktree's copy).

**A dataset's rows.** Each changed, added or removed dataset's
`DatasetImpact.rows` is a `RowDiff { before, after, keyed?, note? }`: each
side's rows read through that side's own overlay (up to 10 000 — the read
gateway's own cap, `MAX_ROW_LIMIT`; `ROW_DIFF_LIMIT`). When both versions
declare the same non-empty `key`, it is a merge-join by key —
`KeyedDiff { key, added, removed, changed, samples }`, up to 20 sample
rows in key order, each with its key and its row before and after (key
order is SQLite's: numbers by value, then text). Otherwise — past the
limit, or when a side's key has a NULL part or repeats (one row would
stand for several; the key test runs only at publish) — the review gives
counts with a note that says why (or that a side's query failed). An
unchanged dataset has none. These rows are the review's only reads of
dataset data; nothing is written.

**A collector's outputs.** Each derived collector (Starlark,
jaq) whose script or declaration changed is dry-run on the same inputs
in both versions: each version's intent-example fixtures that name it
(`fixtures/<example>.yaml`, `when: { collector, rows }`), the latest
five events its `on:` trigger matches, else its `input:` query once. The
runner is `collector_runner::dry_run_collector`: each version's own
script text, its `input:` read through that version's own overlay,
storing nothing, as a pure run. A script that names an `ai_*` builtin
is refused before it runs (a review never spends or sends). The run has
the command scripts' `COMMAND_SCRIPT_BUDGET` (5 s, not a live
collector's 120 s: someone waits on a review).
`CollectorImpact.outputs` holds each input's `Ran { counts, rows (20 per
entity), error? }` before and after. An exec or read collector is never
run and says so in `not_run`. A collector whose entry's text changed
though its declaration did not is `changed` with `script_changed`, and
says "its script changed" even when there is nothing to run it on.

**An effect's reactions.** `ImpactReport.effects` lists each
effect by id (`EffectImpact`) with its trigger before and after
(`EffectTrigger { on, filter, needs }`; a script-only change is
`changed` too). When it changed, it also has `outputs`: each input — both
versions' fixtures that name it (`when: { effect, event, answers? }`)
and the latest five events of its `on` types — composed by each version
that reacts to it (`effects::dry_run` through that side's overlay,
nothing runs): `Composes { commands, skip?, error? }`. The lines read
"Effect x: added — on t where k = v", "Effect x: on … → on …", "Effect x
on fixture basic: runs [a] → skips (why)".

**A bounded review.** Script dry runs — collectors' and
effects' — share one deadline, `REVIEW_DEADLINE` (60 s, from the start
of `impact`; `impact_within` takes another). Past it, no further input
runs, and each collector or effect that did not get through its inputs
is in `ImpactReport.out_of_time` (`collector <id>`, `effect <id>`), with
the line "Out of time: collector a and effect b — the review stops
running scripts after 60s". Only the later side is *checked*
(`prepare`: commands and their examples, components, advisories, page
shapes); the earlier side is only *read* (`read_side`: its datasets'
overlay and its pages dry-run on it) — what it was, not whether it
was right.

**`oxplow extension check <name> --impact [--against <rev>]`.**
The same review on the CLI: `oxplow_sdk::check(…, against)` — the check
and its impact with one throwaway oxplow's command registry. It loads
the extension at git `HEAD` (or `--against`) — `extension_tree_at`
through a `Trees` over the VCS alone — as `before` and the working tree
as `after`, and runs `extensions::impact_between`. Each side is on its
own overlay over the project's database read read-only (else the
check's empty one) — a review, the one place a check reads the project's
data — so the check **writes nothing** — the database file is
byte-identical after. Text output adds `impact against <rev>:` and the
report's lines; `--json` adds `impact` (the `ImpactReport`) and
`against` to the check's JSON.

**On an effort's review.** The diff page (`oxplow-foundation/diff-view`)
has an "Extension Changes" table, empty unless the range touched
`.oxplow/extensions/**`. Its `extensions` loader reads the live dataset
`v_extension_change` (pinned to `base` and `head`, each a `v_change`
revision spelling; columns `name`, `change`, `errors`, `lines`,
`page_changes`, `stream_id`). The server's
`extensions::extension_changes_between` names each extension whose files
differ between the two revisions (`Trees::diff`), loads both versions as
those revisions hold them (`extension_tree_at`) and reviews them with
`impact_between`. Each row has the name, the change, the problems, the
report's lines and the pages that differ (`page_changes`). A removed extension has no report. The pages'
before/after text is not shown (the changed file's diff has it). A failed
review is a row's problems, never a spinner that does not end.
