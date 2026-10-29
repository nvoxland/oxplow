# Usability rules


Things I keep forgetting. Read this before adding any UI.

> **IA redesign — phases 0–7 fully shipped.** Modal `ConfirmDialog`
> and `PromptDialog` chrome was retired in favor of inline patterns;
> per-row actions live on **right-click** menus (the redesign's kebab
> `⋯` buttons were reverted — see "Per-row actions" below); per-stream
> and per-thread settings
> ship as Page tabs (the `stream-settings` and `thread-settings` pages); new-
> stream and new-task flows ship as Page tabs (the New Stream and New Item
> page forms); snapshot- and commit-detail Slideover wrappers
> (`SnapshotDetailSlideover`, `CommitDetailSlideover`) cover the
> cross-page open path. The rules below describe the redesigned
> target. Phase 7 (density + visual polish) details live in
> `.context/theming.md`'s Density section; the per-phase migration log
> lives in `.context/pages-and-tabs.md`. Plan:
> `/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`.

## Markdown is one component

Wherever markdown is read or written — a wiki page, a work item's body,
an effort summary, a comment and its composer, a form's markdown field,
a page's `markdown` viz — it is the one `Markdown` component
(`.context/markdown.md`), so links, mermaid, code blocks with Copy,
tables and comments behave the same everywhere. A surface never renders
markdown itself, nor edits it in a `<textarea>`.

## Forms

- **A form that edits what is saved saves only when changed.** Save is
  disabled until a value differs from the saved one; Discard (and
  Escape) puts the saved values back. What the person typed is a draft
  in the tab's history state (`useDraft`; a page form keeps it as
  `form:<page id>`), so Back, Forward and a restart give it back, and a
  re-read of the saved values never wipes it. A secret (a key, a token)
  is masked, never kept in a draft (history state is written to disk),
  and blank means "keep the saved one". `SchemaForm`'s `mode: "edit"`,
  `draft` / `onDraft` and `fields` specs do all of this; a page form
  declares it (`form: { mode: edit }`, `fields: [{ name, secret }]`).
- **Edit-X-in-place actions are inline, not modal.** Click the
  displayed value to swap to an input; Enter commits, Escape reverts,
  blur commits unless Escape was pressed. The shared helper is
  `apps/desktop/src/components/InlineEdit.tsx`; `TaskDetail`'s
  `EditableField` and `WorkGroupList`'s `InlineItemRow` are older
  hand-rolled equivalents — copy whichever is closest. The cancel
  latch must be a `useRef` (state updates are async; the blur fires
  on the same tick). Use `multiline` for textareas (Cmd/Ctrl+Enter
  commits; Enter inserts newline). Use `allowEmpty` to permit
  clearing.
- **Tiny prompt strips render inline at the top of the owning
  panel** for "+ New file" / "+ New folder" / Rename flows where the
  trigger comes from a row's right-click menu rather than a row that
  already shows the editable value. See `InlinePromptStrip`
  (`components/InlinePromptStrip.tsx`: one or more fields, a field may be
  `multiline` — Cmd/Ctrl+Enter submits there; the owner dismisses it, so
  a failed run keeps what was typed, and `error` is why, under the
  fields). A page's form action whose form, less its `fixed` fields, is
  one text field is one too, at the top of the page (the action prompt strip): a Rename… on a Files
  page is the Project panel's strip, never a one-field slideover. Same Enter-submits / Escape-cancels contract;
  the strip is dismissed by the panel's local `pendingPrompt` state.
- **Form-shaped flows that warrant a focused workspace use a page tab
  or a slideover, not a centered modal.** The "+ New" flows ship as
  Page tabs (the New Stream and New Item page forms, the `Stream/Thread`
  settings pages); cross-page detail openings (snapshot, commit,
  branch rename, file commit) ship as Slideovers. Do not
  add new modal call sites; route new flows through pages or
  slideovers. The page pattern to copy is
  `apps/desktop/src/pages/PageHost.tsx` (a page file; Settings is a switch page) — full Page tab, no backdrop.
- **A long page splits into pages, and a link lands on its section.**
  Settings is one page (`settings.yaml`, a `switch` over its areas: overview, Agents,
  Language Servers, Extensions, Data, Integrations, AI), an area the section
  in view,
  each its own URI (`settingsRef(area, section)` → `oxplow:oxplow/page?page=settings#area=…`),
  sharing a rail that lists the areas; the launcher lists each area by
  name ("Settings: Agents"). A link that means one place opens its area
  with `section` — one of the area's section ids or an anchor inside one
  (`settings-data-delivery`) — and the page scrolls there. A page of
  four sections or more lists them in its rail (Contents), those whose
  heading is hidden (`hide_title`) too. A declared document does the same with `view:
  <param>`: the one section in view is the page, the others are its rail,
  a change of section is pushed (Back returns), and the launcher lists
  each section by name. On any page `#section=<id>` lands on that
  section. A page that shows state from outside any dataset
  refreshes on the event that says it changed (Data on `extensionsChanged`; Integrations'
  page re-runs when the `v_provider_*` models change). Integrations
  also re-reads when an instance's health is recorded (`v_contribution_health`): Enable's own `configChanged` can be read
  before the instance has started, and nothing else says it finished
  (tsk1053).
- **A re-read never wipes what the person typed** (tsk1054). A host
  re-reading its data hands a form fresh copies of the same schema and
  saved value, so a form resets its fields on the *content* changing, not
  the object: `SchemaForm` keys on `JSON.stringify` of both, and a row
  holding a draft beside it (`IntegrationRow`'s `config`) does the same.
  Otherwise any event that re-reads — a deploy, a config change
  elsewhere, a health record — silently drops unsaved input, and the next
  action sends the old value.
- **A command's form follows its input schema** (`SchemaForm`): a
  string declared `contentMediaType: text/markdown` is a text area where
  Enter adds a line and Cmd/Ctrl+Enter submits; a field a page gives
  `choices` is a select of their labels, under their groups.
- **Never call `window.prompt()`.** The Tauri webview blocks it —
  it returns `null` synchronously without
  showing anything, so any code path gated on its return value
  silently no-ops. Use `InlineEdit` (for
  click-to-edit) or `InlinePromptStrip` (for new-X flows that need a
  target-path entry) instead. `window.confirm` / `window.alert` block
  the renderer, and `no-blocking-dialogs.test.ts` fails on any call
  (tsk630: the Git dashboard's push / merge / rebase now ask with
  `InlineConfirm`); use `InlineConfirm` for destructive actions on a
  row/button and `showToast({ message, onUndo })` for fire-and-undo
  destructives that aren't tied to a specific row. A destructive command run
  from a menu (which closes, leaving nowhere to ask inline) goes through
  `personCommands.run`: its `NeedsConfirmation` shows the shared
  `CommandConfirm` (`PersonCommandConfirm`, mounted once in `App`). The
  task page's and wiki page's rail Delete use `InlineConfirm`; the wiki
  pane's right-click Delete uses `personCommands`.
- **What needs the person isn't in the rail.** One status-bar
  bell (`components/Alerts/AlertsIndicator.tsx`) counts everything that
  needs them — red while something failed, the accent while decisions or
  notices wait, quiet otherwise — and opens the **Alerts page**
  (the `alerts` page, the foundation's): Needs your
  decision (each proposal's card, Approve / Decline), Hints for you,
  Failed operations (each opening into how it ended), Undelivered events
  and Failed reactions with Retry / Discard (`oxplow.delivery.*`,
  `oxplow.effect.retry`, as page actions that ask first), and Extensions
  with problems. A firing panel badge is the panel's own: it is not on the
  page and the bell does not count it. A toast announces each new item once
  (`useAlertToasts`; what's there in the first seconds after the app opens
  counts as seen) and offers only **Review** — never Approve: consent is
  given where the preview is. A badge's Review opens the panel's page.
- **Async-op failures don't `alert`.** Record them:
  `recordOpError({ label, command?, stderr?, stdout?, exitCode?,
  message? })` (`opErrors.ts`) reports it to the daemon, fire-and-forget,
  as `oxplow.ui.report_error`, and the record is the dataset,
  `v_op_error` — the client keeps nothing, so the list survives a reload
  and the agent reads the same rows (the always-on agent text and the
  `oxplow-codebase` skill say so, so "can you see that error?" needs no
  screenshot). A failure is one of the things that need the person
  (below): a toast as it arrives ("<label> — Review"), the status bar's
  bell counts it (red), and the **Alerts page** lists it under Failed
  operations (`v_op_error`, those not dismissed), each row expanding
  (`columns[].expand`, the `alerts-operation` page) into how it ended and
  its output. Mark Seen runs `oxplow.op_error.mark_seen` (it stops being
  red), Dismiss `oxplow.op_error.dismiss`, "Clear failed operations"
  `dismiss_all`. For
  ops that already have a page focus when they fail (e.g. `runOp` in
  GitDashboardPage), call `onOpenPage(alertsRef())` after recording so
  the person lands where it's shown. A report, or a read of the list,
  that fails is logged with `logUi`, never recorded as another op error
  (it would loop).
- **Settings → Extensions shows the grant, not only the verdict.** Each
  row lists the extension's permissions, and what a deploy, roll back or
  enable would add is shown in the confirmation before it runs
  (`CommandConfirm` lists one line per added permission; `process` says
  that oxplow does not confine the program). **Roll Back and Remove are
  destructive, so they are on right-click** like the other row actions,
  and a row action that adds a permission asks inline, where the person
  clicked.
- **Settings → Extensions shows each extension's last run.** One line under
  the row from `v_extension_run` (`latest = 1` per contribution): the **worst
  current warning** (denied, then stale, then failed; the newest of equals) so a
  failing contribution isn't hidden behind another's later ok run, else the
  most recent run: "Last run: ok · collector pull · <when>".
  Failed, denied and stale are in the warning color (`--severity-medium`) and
  say why — a failed or stale run's `error`, a denied run's refusing decision
  from `decisions` ("host evil.example: it isn't in the grant"). An extension
  that has never run shows no line. `lastRuns` in `extensionRowModel.ts`; the
  line re-reads when a run commits (`useRerunOnChange` on the read's datasets).
- **Settings → Extensions says what each script is given.** Under the
  row, one muted line per Starlark script that has services: its name, then
  the service titles ("Collector triage — Read the captured tree · Ask a
  model (needs `ai.compute`)"). A service the script uses without naming it in
  `needs` says "(not in its needs)". The install/update review lists the
  same lines above the impact report. `serviceLines` in
  `extensionRowModel.ts`.
- **Every `<button>` needs an explicit `type`.** HTML defaults
  `<button>` to `type="submit"`, which silently submits any enclosing
  form on click. Use `type="button"` for every action button; use
  `type="submit"` only on the form's primary action. Don't rely on
  the default — it's a tripwire.
- **Enter submits.** Any form with a primary action must submit on
  Enter from any single-line input or select when all required fields
  are valid. Use a real `<form onSubmit=...>` wrapper; the browser
  handles single-line Enter for you. For multi-line textareas, Enter
  inserts a newline and Cmd/Ctrl+Enter submits.
  **Exception: chat prompt boxes.** A chat thread's prompt box
  (`components/chat/ChatPromptBox.tsx`) follows the chat convention —
  Enter sends, Shift+Enter inserts a newline, Escape stops a running
  turn — because it is a conversation, not a form. While a turn runs,
  typing still works but Enter/Send do nothing (prompts are never
  queued).
- **Escape cancels.** Inline edit fields and inline-confirm pairs
  revert on Escape. The legacy modals that haven't migrated yet still
  close on Escape via their own keydown listener.
  A control that handles Escape (a form, a confirmation, Keep This)
  stops its propagation, so a container that also listens — the Answers
  strip collapsing — doesn't fire on the same key.
- **Disabled submit button when invalid** rather than erroring on
  submit. Show required-field hints inline.
  `SchemaForm` also takes `fields` (show only these top-level fields,
  in order; the rest keep their `initial` value and are still
  submitted), `fixed` (shown as read-only text, `<prefix>-<field>-fixed`,
  submitted unchanged, never focused) and `serverErrors` (a host's
  message per top-level field, under the field like a client error and
  cleared when that field is edited; one for a field not shown appears
  once above the submit button as `<prefix>-form-error`). Server errors
  never disable submit; client errors still do. All three are keyed on
  their content, so a re-read keeps what was typed.
  A command's refusal that names a field (`INVALID` with `field`, on
  `IpcCallError.field`; `fieldErrors`) is shown that way by every form
  that runs a command — a form page, a page's form action (`form: true`,
  in a `Slideover` — or a prompt strip when its form is one text field —
  never a modal; success closes it) and, through the
  bridge's `field`, a custom component's own form — never only in the
  error rail.
- **Autofocus the first input** in any inline edit / prompt strip
  when it mounts (and select existing text so the user can replace
  it with a single keystroke). A *page* that opens on its own (not from
  the person's own action) focuses its input only when nothing else has
  focus: one that mounts late — the New Session page as a thread loads —
  must not take the keystrokes of a launcher the person already opened
  (the New Session form doesn't take focus at all). The launcher starts a fresh query only when it opens, and
  takes the focus whenever it shows (on a fresh window that's once its
  stream has loaded, just after it opened): what a person typed is never
  wiped by data arriving. And it keeps the focus while it is open: when
  something else takes it from outside (a restored tab or terminal mounting
  as a reloaded window settles), the input gets it back on the next frame
  (`QuickOpenOverlay`'s `onBlur`); focus moving inside the launcher is left
  alone, and a click outside closes it first. If keystrokes are lost again
  after a reload, log `document.activeElement` in that `onBlur` to name the
  element that took focus; the fix was made without seeing it in a failing
  `settings/pages.spec.ts` run.
- **"Save and Another"** for repetitive-entry flows (see the New Work
  Item form): a form page declares `again: { label, keep }`, a second
  button that files and restarts the form with the `keep` fields (state,
  thread, parent, the list's own fields) carried over and the rest
  cleared, so a series of similar items isn't re-picked. Enter keeps the
  primary submit.

## Copy names things as a person knows them

- **No internal ids, codes or placeholders in what a person reads**.
  A thread by its title ("The agent in “Fix the cart”"),
  never `thr1`; a command by its label, its id only on hover; a
  capability by its title (`capability_title`), an area by its name
  ("Version control", `areaLabel`), not
  `vcs`; no plan codes ("(P5.E1)") in dataset docs (`dataset_docs_carry_no_
  plan_labels`); no `TODO:` scaffold text a person sees (Keep This's
  `my-pages` gets a real description); counts agree in number ("1
  note"); a status says what happened ("Recorded facts", not "0 rows").
- **The precise technical term, not a friendlier synonym.** oxplow's
  users are software developers, so UI copy uses the same terms the code
  and docs use — capability, extension, contribution, provider, scope,
  access, effect — with the meanings in
  [extensions.md](./extensions.md) "Terms". No "pieces", "plugins" or
  "add-ons" for them; a "provider" says which kind (an external provider,
  an AI provider) where the screen doesn't make it obvious.

## Agent proposals

- **Approving is the confirmation.** An effect's run that asks, or a
  handler's consent check (a deploy that adds permissions), waits as a
  proposal; its
  `ProposalCard` (`components/Proposals/ProposalCard.tsx`) shows who
  proposed it, what it would change (a composite's commands) and Approve / Decline. Approve runs it as the
  person with no modal; a destructive one is tinted and its Approve is an
  `InlineConfirm` (the per-row destructive rule: one click arms, the
  second runs). A failed decision shows its error on the card, next to
  the buttons. Cards appear in the **Approvals** rail panel. Testids:
  `proposal-<id>`, `proposal-approve-<id>` (a destructive one's:
  `proposal-approve-<id>-trigger` / `-confirm`), `proposal-decline-<id>`,
  `proposal-error-<id>`, `rail-alert-proposals`.
- **A proposal also waits where the conversation is** (P9.A3). Above an
  agent session's terminal, the page's `approvals` section shows the
  thread's pending proposals. A chat transcript
  shows the card under the tool call that made the proposal
  (`proposalOfTool`); once decided, a muted line stays there saying what
  became of it — "Approved by you — it ran.", "Declined by you — nothing
  ran.", "Replaced by a newer proposal." —
  so the transcript keeps what the agent asked. An approved one says it
  ran only once its run is recorded (`audit_id`): until then it says
  "Approved by you — running…", since an External command is claimed
  approved before it runs and goes back to waiting if the run fails
  (tsk858). Only the thread's own
  proposals show: a ref the transcript merely quotes shows nothing. The
  Alerts page stays the cross-thread list (its proposal cards).

## Menus a page or row gets from extensions

- **Order is fixed.** A page's nav bar's ☰: oxplow's own first (Ask the Assistant, then
  Add to Context and the standard actions, in their order — the same spot on every page),
  then the page's and its extensions' own sorted by label, a submenu by
  its name (an action's `submenu` and a command's `ui.group` of the same
  name share one), then Open With and the panels
  (`.context/pages-and-tabs.md` "The nav bar is core's"). A row's right-click menu: Ask the Assistant, the page's row
  actions, then a separator and one submenu per `ui.group` with the
  commands about the row's ref (`refCommandMenuItems`). A Board card:
  Move To, then the same tail. The Navigator's thread menu ends with the
  same tail, the commands about that thread. A submenu opens on hover
  and on click; a click never closes the one its hover opened. Each runs as any offer does (asking first
  when the command asks). Testids: a row menu's group
  `menu-item-ref-commands-<group>`, the ☰'s submenu
  `menu-item-page-submenu-<name>`, a command in either
  `menu-item-ref-command-<command id>`.

## A custom component is marked

- A `component` node (an extension's own web component, sandboxed)
  renders under a small **custom** badge, so the person knows the view
  isn't oxplow's. Its confirmations appear in oxplow (`CommandConfirm`
  below the frame), never inside the frame. When it can't start, a muted
  line says so and the page's table shows.
- A **custom page** (a page whose `view:` is one `component` node, `pages/PageComponent.tsx` `ComponentPage`)
  keeps oxplow's nav bar and the **custom** badge; the frame fills only
  the page body. Its title and actions are data the frame sends
  (`setTitle`, `setActions`) and core draws in the bar — never buttons of
  oxplow's look inside the frame for page-level actions. A form it asks
  for (`openForm`) is oxplow's slideover, with Enter submitting and
  Escape closing as every form.

## A replaced component is marked, and falls back to oxplow's

- Where an extension's page stands in for a core component (the Board's
  cards — `Replaced`, P9.A1), a small **replaced by <extension>**
  badge says whose it is; the page's own chrome stays oxplow's. When the
  replacement can't load, oxplow's own component shows, under a muted
  line saying whose couldn't load and why — never an error instead of
  the page, and never the page's table. A person switches back for good
  with "Always use oxplow's own <component>" on Settings → Integrations.
  Testids: `replacement-<target>`, `replacement-badge`,
  `replacement-fallback`, `integrations-replacement-<target>`,
  `integrations-replacement-off-<target>`.

## Decorations are additive

- An extension's decorator adds a chip to a page's header (after the
  page's own) or a badge after a page cell; a page or row is complete
  without them, and a decorator that fails shows nothing. They're labels
  with an extension's name in their tooltip, never actions.

## Destructive actions

- **A button that runs a command asks when the command's spec does**
  (`SpecConfirm`, `apps/desktop/src/components/SpecConfirm.tsx`; tsk898):
  never a second rule in the page — while the spec loads it asks.
- **Per-row destructives use `InlineConfirm`** at
  `apps/desktop/src/components/InlineConfirm.tsx`. First click on the trigger
  swaps to a `[Confirm] [Cancel]` pair in the same horizontal real
  estate. The Confirm button auto-focuses; Escape, a press outside the
  pair, focus moving elsewhere, or Cancel reverts. Focus moving to an
  element *around* the pair (the tab or row it sits in) doesn't revert
  it: WebKit, the desktop window's engine, doesn't focus a clicked
  button, so a click on Confirm moves focus to the nearest focusable
  ancestor first — reverting on that swallowed the click (the session
  tab's Close session did nothing in the app while Chromium tests
  passed). Check a focus-dependent control in the WebKit e2e project
  (`tests-e2e/specs/components/`). Examples in tree: Restore button on each
  file row in `SnapshotsPanel.tsx`'s detail pane; Delete button on
  `WaitPointRow.tsx`; Force-delete button in `BranchPicker.tsx`'s
  manage flow.
- **Non-row-anchored destructives fire immediately and surface an
  Undo toast.** Use `showToast({ message, onUndo })` from
  `apps/desktop/src/components/toastStore.ts`. The toast auto-dismisses after
  ~7s and the [Undo] button calls the supplied callback. Mount the
  `<UndoToastStack />` once near the app root (already done in
  `App.tsx`). When the action is genuinely irreversible (delete a
  task permanently) push a toast without `onUndo` so the user
  still sees confirmation feedback even if they can't undo. Don't
  block the renderer with a centered confirm modal.
- **Closing an agent session's tab closes the session at once**, with an
  Undo toast: one click on its × runs `oxplow.agent_session.close`
  through `personCommands`, and Undo runs its inverse,
  `oxplow.agent_session.reopen`, which brings the tab back and resumes
  the saved conversation. Rename… is on the tab's right-click menu; it sets
  oxplow's name for the session (the strip says so, and what clearing it
  falls back to — the harness's own name, shown as the placeholder), with
  an "Also rename it in <harness>" box only where the harness can.
- **A tab whose page holds unsaved edits says ●** before its label (a
  file, a wiki page's draft — any page body that calls `useTabEditing`).
  File → Save (Ctrl/Cmd+S, `when: pageDirty`) saves the page shown
  through its own Save, and the tab cap never closes such a tab.
- **Closing a dirty file tab** is fire-and-undo: the close completes
  immediately and a toast offers Undo (which restores both the saved
  buffer and the unsaved draft). See `App.tsx` →
  `handleCloseOpenFile`.

## Actions that reach outside oxplow

Some actions do something oxplow can't take back or doesn't own: an
external provider's service is written to, a browser sign-in starts, an effect
runs again. The rule is the destructive one's — the person's second
click is the confirmation — plus saying what is about to happen.

- **A retry says what it might repeat** (P9.D4). Settings → Data →
  Delivery lists an effect's failed reactions with why they failed;
  **Retry** is an `InlineConfirm` whose title says a step outside oxplow
  may already have run and that retrying sends it again
  (`reaction-retry-<effect>-<event>-trigger` / `-confirm`).
- **A count a person should see comes before the confirming click**
  (P9.D5). A command's confirmation shows its summary and input, not
  something computed; so **Backfill…** on a deployed effect's row first
  reads the plan (`oxplow.effect.backfill_plan`), says how many events the
  effect never reacted to and that it may call outside oxplow for each,
  and offers "Run on N events" beside Cancel. Escape cancels; nothing to
  do says so, with Close (`effect-backfill-<key>`,
  `effect-backfill-ask-<key>`, `effect-backfill-run-<key>`).
- **Signing in happens in the person's own browser** (P9.B3). A
  credential obtained by signing in has no value box: its row shows
  where it stands ("Not signed in", "Signed in", "Signed in until …",
  "Sign in again: …"), **Sign in** / **Sign in again**, and **Sign out**
  (an `InlineConfirm`) when signed in. Sign in opens the service's page
  in the system browser — never oxplow's in-app window — and the row
  says "Finish signing in in your browser…" until oxplow hears how it
  went; a failure is shown on the row (`sign-in-<instance>-<name>`,
  `sign-in-button-…`, `sign-out-…`).
- **Adding an instance is a small form under the list** (P9.B6): the
  external provider (a select only when there is more than one), a name, and
  whose it is ("This project's" / "Mine, in every project"). Enter adds,
  Escape clears, and what's wrong with the name is said before Add can
  be pressed. **Remove** is an `InlineConfirm` on a named instance; an
  external provider's own instance is turned off, not removed.

## Links open where you ask them to

Any link that resolves to a page — wiki, task, file, directory, commit —
supports the browser conventions: **Cmd/Ctrl-click or middle-click opens
it in a new tab**, plain click navigates in place. The right-click menu
carries the same pair (Open / Open in new tab) so the gesture is
discoverable without knowing the modifier.

The rule that keeps this honest: every page-bearing link resolves
through one `linkRef(parsed)` and one `openRef(ref, { newTab })` call
(`components/Markdown/links.ts`, `followLink` and `linkMenu`, which the
markdown viewer and editor both use). Per-kind click branches are
how it drifted before — task links honoured the modifier while file,
directory and commit links silently ignored it (tsk265). A new link kind
gets the behaviour by being added to `linkRef`, not by growing
another branch.

## A page changes its own view, it doesn't open itself

A filter, a mode, a pick or "Show every snapshot" on a page is that page at
another view: it writes its view params (`useViewParam`, `useViewSelection`,
`.context/pages-and-tabs.md` "Page state"), and the page stays mounted —
its scroll, open groups and drafts stay. A filter (a toggle such as
"Show every snapshot" too) replaces the tab's entry; a pick or a step the
person will want Back from (another Settings area, another diff endpoint)
pushes one. A mode is a
param, never a second page name. A list's cursor (↑/↓ before Enter) is
not a pick and isn't in the address.

## Hover reveals, click rearranges

> **These rules are enforced by code, not just written down here.** The
> open/close state machine for a slide-out glyph strip lives in
> `apps/desktop/src/components/useSlideoutStrip.ts`, with the shared
> bottom-pinned toggle in `SlideoutChevron.tsx`. Both the far-left
> `Navigator` (streams + threads) and the Terminal page's
> `TerminalTabStrip` run on it. **A third strip adopts the hook — it does
> not hand-roll a fourth copy of the timer, the listeners, and the guard.**
> The terminal strip was hand-written "modeled on the far-left Navigator"
> and inherited every one of the bugs below, which is why the behavior is
> now shared code rather than a convention (tsk269, tsk270).

- **Hover must never open anything that occludes an adjacent panel.**
  Hover is involuntary — the pointer crosses your component on its way
  somewhere else. So it may reveal information *in place* (a tooltip, a
  highlight, a row's hidden affordance), but expanding a surface over a
  neighbor requires an explicit click. The Navigator learned this the
  hard way (tsk269): its 280px panel opened on a zero-dwell `mouseEnter`
  and covered ~92% of the rail HUD immediately to its right, so simply
  drifting left on the way to a rail row buried the row you were aiming
  at. A dwell delay only makes an involuntary action *slower*, not
  voluntary — the fix was moving the expansion onto a click and leaving
  hover with the tooltip.
- **Never gate a control on hover alone.** Whatever hover reveals needs a
  click/keyboard route too, and an affordance that's visible before you
  hover. Corollary: an expandable strip needs a persistent chevron, not
  just a clickable dead zone — dead space evaporates exactly when the
  list is long, which is when you most need the panel.
- **Closing on pointer-leave is a geometric test, not `mouseleave`.** An
  absolutely-positioned panel that covers a sibling is still inside its
  own wrapper's DOM subtree, so the wrapper's `mouseleave` never fires
  while the pointer is over the covered region — the panel strands itself
  open on top of the thing it's covering. Compare the pointer against the
  panel's `getBoundingClientRect()` on a document `pointermove` instead.
  Keep a short grace delay (~180ms) so crossing a seam doesn't snap it
  shut.
  **Only after the pointer has been in it.** A panel opened from somewhere
  else (the title bar's stream name, above the Navigator) starts with the
  pointer outside; counting that as "left" flashed it open and shut on the
  first mouse move. `useSlideoutStrip` arms the passive close once the
  pointer has entered the panel.
- **Passive closes yield to an in-flight form; explicit ones don't.**
  Pointer-leave and background-click must not discard a rename or a
  half-typed new-item entry. Escape, the collapse control, and a press
  outside the surface are the user actively dismissing — those go
  through.

## Per-row actions (right-click menus)

> The IA redesign briefly moved per-row actions onto visible kebab `⋯`
> buttons; we reversed that. Right-click is discoverable enough for
> anyone who expects it and the kebab ate row space, so **per-row
> actions are right-click only again** — no `⋯` buttons on rows. (The
> old `Kebab.tsx` primitive is deleted.) A page's row actions
> (`row: true`) are that menu, and nothing else: a declared page gets no
> row buttons either.

- **Right-click a row to open its action menu.** The shared hook is
  `apps/desktop/src/components/useRowContextMenu.tsx`:
  - `useRowContextMenu(items, header?)` — bind the items when the row is
    its own component; spread `onContextMenu` / `onKeyDown` and render
    `{menu}`. `header` titles the menu with what it acts on
    (`context-menu-header`) — the Navigator heads every stream / thread
    menu with its name, since a strip glyph shows only initials.
  - **An item has one menu wherever it shows.** The Navigator builds each
    stream's and thread's menu once (`streamMenu` / `threadMenu`) and
    binds it to both the strip glyph and the panel row; an action whose
    inline field lives in the panel (Rename, Add thread) opens it. A
    surface outside the Navigator that names a stream or thread (the title
    bar) asks for the menu over `navigator-bus` (`requestNavigatorMenu`,
    opened with `useContextMenu().openAt`) rather than building its own.
  - `useContextMenu()` — call once in a parent that renders rows in a
    `.map()` (a per-row hook can't run there); each row does
    `onContextMenu={(e) => open(e, items)}` and the parent renders
    `{menu}` once.
  Both open the same `ContextMenu` popover with the same `MenuItem[]`
  payload. Some surfaces (Plan task rows, the blame gutter)
  instead keep a single parent-owned `ContextMenu` and have each row's
  `onContextMenu` call an existing `onOpenMenu({…, x, y})` opener — pass
  the cursor coords (`new DOMRect(clientX, clientY, 0, 0)` when the
  opener wants a rect).
- **Keyboard parity is required** — right-click is mouse-only, so the
  hook's `onKeyDown` opens the same menu on the **Menu key / Shift+F10**
  for the focused row. Wire it on focusable rows; keyboard-first users
  must never need the mouse. (A page row also covers this with its
  actions' `key:`s.)
- The `ContextMenu` popover renderer at
  `apps/desktop/src/components/ContextMenu.tsx` is unchanged. Prefer the
  hook over a raw `onContextMenu` so suppression + keyboard parity stay
  in one place.
- **`menu-item-<item.id>` testids** stay on every button inside the
  shared `MenuList` — the `MenuItem.id` becomes the testid suffix
  (e.g. `menu-item-task.delete`, `menu-item-task.rename`). To drive a
  row menu in a test, dispatch `contextMenu` on the row (e.g.
  `fireEvent.contextMenu(getByTestId("navigator-thread-row-<id>"))`)
  then click `menu-item-<id>`.
- Close on outside click, scroll, window resize.
- **The native WKWebView context menu is globally suppressed as a
  backstop.** `installContextMenuSuppressor()` (in
  `apps/desktop/src/context-menu.ts`, mounted once from `App.tsx`) cancels
  the OS-default right-click menu (Look Up / Translate / Copy / Share /
  Inspect Element / Services) so it never leaks on bare surfaces that
  have no row menu. (Row menus cancel it locally and open ours.) It
  exempts text inputs / textareas, contenteditable (Tiptap), Monaco
  (`.monaco-editor`), and the terminal (`.xterm`) so right-click
  copy/paste and the editor's own menu still work there. The decision is
  a pure `shouldSuppressContextMenu` predicate over an
  ancestor-descriptor chain (unit-tested without a DOM); add new exempt
  surfaces there.

## Actions are data; core draws them

- **A page's actions are `PageAction[]`** (`src/kit/actions.ts`), drawn in
  the nav bar by core — never a page's own buttons in its chrome. The bar
  shows only the primary action and the `prominent` ones as buttons; the
  rest are items of its ☰ menu, so the bar stays short whatever a page and
  its extensions add. Mark an action prominent only when it is what a
  person comes to the page to do (a list's New, a
  timeline's Today/Fit). A button with an `icon` is the icon alone, its
  label the hover; give a prominent action one when the icon says it
  plainly. A section's actions are `PageAction[]` too, at its heading. A
  primary action is the accent button; one per place.
- **A page's title is in the nav bar, always**, with its icon; the body
  starts with its first section and draws no heading of its own. A long
  title ends in an ellipsis and the full text is its hover.
- **A section is a heading over a rule**, no box (`kit/Section`). A card
  is a dashboard tile's, not a page section's. Every section has a title
  (it is its Contents entry); a heading that would look wrong is hidden
  with `hide_title: true`, never left off.

## Proposals are drawn once

An agent's pending proposals are drawn by the kit's `Approvals`
(`src/kit/Approvals.tsx`): `ProposalCard` is its card, `ProposalList` draws
proposals a page already read (Alerts, a Settings row's compact card) and `Approvals` reads them itself, which is what
`viz: approvals` renders. A page never draws its own Approve / Decline; it
mounts the kit's, so a page moving to the kit's viz (`viz: approvals`, params
`thread` / `setting`) changes where the cards sit, not the cards.

## Collapsible page sections

- **A page with stacked sections gets them from the shared primitive**, not
  hand-rolled state — three parts in
  `apps/desktop/src/components/CollapsibleSections.tsx`: `CollapsibleSections`
  (state provider) and `SectionCollapseControls` (the Expand all / Collapse all
  pair); the section itself is the kit `Section` with `collapsible` (and
  `defaultCollapsed`), which reads the provider through `useSectionCollapse`.
  Pure state + persistence live in the sibling `sectionCollapse.ts`
  (unit-tested). Adopters: none now the Metrics page is a page (a grouped page list folds its headings itself).
- **The page places the controls; they are not pinned above the sections
  (tsk86).** `SectionCollapseControls` reads state off context, so it can render
  anywhere under the provider. Recorded Metrics puts it in the **details rail**
  beside the filters — the rail is the page's control panel (a page's
  too: "A page's controls are in its rail"). It **self-hides**
  when no sections are registered (loading / empty), rather than showing two dead
  buttons.
- **The provider renders `children` bare — no wrapper element — on purpose.** It
  wraps the whole `<Page>` so context reaches *both* the rail and the body
  (`rightRail` is created by the page but rendered inside `Page`'s subtree, and
  context follows the render tree, not the creation site). Any wrapper here would
  sit between the tab body and the page chrome's `height: 100%` column and break
  it.
- **Still NOT a `Page` prop (tsk84).** `Page` is chrome: it renders `children`
  opaquely and has no idea what sections exist, so a page-layout flag couldn't
  draw the controls without the sections registering themselves anyway — the flag
  would only say "the thing I'm already doing is allowed". The page composing
  `<SectionCollapseControls />` where it wants is simpler and more flexible.
  (Note this is *not* the `Page` `actions` slot either, which would land the pair
  in the rail's **panel header** rather than in the rail body.)
- **Sections default to expanded**; only the *collapsed* set is stored
  (the tab entry's history state, `useHistoryState`: Back and a restart bring it back), so a newly added
  section appears open with no migration. A section with `defaultCollapsed`
  is the mirror: it starts closed and only the ones the person *opened* are
  stored (beside the collapsed ids). Expand all /
  Collapse all handle both kinds.
- **A page tree's branches**: one the result nests is open by default
  (the closed ones, by index path, are history state); a lazy branch
  (`tree.lazy`, a folder loaded on demand) is closed by default and the
  opened ones, by the row's id, are history state — a folder left open
  under a closed one stays open for when its parent opens again. A lazy
  branch's row opens on a plain click or Enter; the disclosure ▸ works on
  every branch; a link or control in the row, Cmd/Shift-click marks and
  the right-click menu keep their meaning.
- **Don't reconcile stored ids against the rendered ones** — the opposite of what
  the rail does for its section ORDER. A page's own filter (Recorded Metrics'
  search) can drop a section entirely; it must come back still collapsed.
  Equally, Expand-all / Collapse-all act **only on what's rendered**, so a
  filtered-out section's state is never silently rewritten.
- **The toggle lives inside the `<h2>`**, not instead of it — the heading stays a
  heading for the document outline, and a real `<button>` gets keyboard
  activation for free. `aria-expanded` reflects state.
- Labels + testids match `HierarchyView`'s existing tree toolbar: sentence-case
  "Expand all" / "Collapse all", `<prefix>-expand-all` / `-collapse-all`, plus
  `<prefix>-section-toggle-<id>` / `-section-body-<id>` / `-group-<id>`. An
  all-button is **disabled when it would do nothing**.

## Commenting on any surface

Comments are not editor-only. Any page region can be commentable by
declaring *what it is* and mounting the generic layer.

- **Comments are a page's.** A page file says `comments: true` and core
  does the rest (`PageHost`, `viz/pageComments.ts`):
  its URI (its tab's identity) is the comments' target, the nav bar gets
  the `CommentNavigator`, the body carries `data-ref` (so plain text and
  the markdown viewer take selections through the app-level layer), a
  markdown editor anchors comments in its own layer, and a `code` viz's
  files take them on their URI at their revision (Monaco's menu has Add
  Comment; a click on a highlight opens its thread). A page that doesn't
  declare it is not commentable. Text inside an editor (Tiptap, Monaco)
  is painted by that editor's layer only: the app-level layer skips it, as
  it skips its selections. The terminal takes no comments (its buffer has
  nothing stable to anchor to), and the app-level layer skips it too.

- **`data-ref` marks a region as a typed "context node."** Its value is
  the canonical URI of the thing, the same as tab ids and the `page_ref`
  graph use ([resources.md](./resources.md)): a file, directory, wiki
  page, work item, commit or finding. Stamp it on the element that *is*
  that thing (e.g. a task row carries
  `data-ref="oxplow:oxplow/work_item?id=tsk42&provider=oxplow"`). Nesting is
  meaningful: a file row inside a commit card yields the chain
  `[file URI, commit URI, …]`, innermost first. Treat
  this as a first-class seam like `data-testid` — spread
  `contextNodeProps(uri)` from
  `apps/desktop/src/components/Comments/contextNodes.tsx`.
- **Selecting text on a context node shows a floating "Add comment"
  button** (`SelectionCommentToolbar`), driven by `useDomAnnotations`.
  It is additive and non-destructive (so a floating affordance is fine,
  unlike destructive actions which stay on the right-click menu), dismisses on a new
  selection or Escape, and reuses the **same** `shouldSuppressContextMenu`
  carve-out so it never appears inside Monaco / Tiptap / inputs / the
  terminal — those own their own comment UX.
- **`DomCommentLayer` is mounted once, at the app level** (`App.tsx`,
  beside the center tab outlet) — NOT per page. A selection only becomes
  a comment when it lands inside a `data-ref` region and outside the
  editor/terminal carve-out, so a single instance safely serves every
  plain-DOM page; a page with no context nodes simply never captures.
  Adding commenting to a new surface is therefore just "stamp
  `data-ref` on the regions" — no per-page wiring. The layer captures
  selections, paints existing comments back onto their context node's
  text via the **CSS Custom Highlight API** (no DOM mutation — critical
  for live React lists), and opens a thread popover when a highlight is
  clicked. The quote is re-resolved against the element's `textContent`
  each repaint (debounced to one per frame), so reordering /
  virtualization just re-anchors. The Highlight API is feature-detected;
  where it's absent the comment still works, just without an inline
  highlight. Surfaces opted in so far: task rows (`TaskGroupList`), the
  commit page (`GitCommitPage` meta → `commit`), and commit-graph rows (`CommitGraphTable` → `commit`, under the
  `git-dashboard` root). The **terminal** takes no comments
  (`.context/terminal.md` "Terminals are rows").
- **Make the text selectable.** Rows are often `userSelect: none` for
  clean drag — set `userSelect: "text"` on the specific label span you
  want commentable (e.g. the task title) so a quote can be anchored
  without re-enabling selection on the whole row.
- **Draggable rows can't be drag-selected** (a mousedown starts the
  drag), so a floating toolbar never appears on them. For those — task
  rows (`TaskGroupList`) — add a
  **"Comment…" item to the row's existing right-click menu** instead. Its
  handler calls `composeForElement(el, label, rect)` (in
  `useDomAnnotations.ts`) to build a `PendingComment` anchored to the
  row's label within its `data-ref` element, then dispatches it via
  `requestCommentCompose` (`comment-compose-bus.ts`). The single
  app-level `DomCommentLayer` subscribes and opens its composer — so the
  create/anchor/paint path stays in one place regardless of whether the
  comment came from a selection or a menu.
- `data-testid`s on the affordances: `selection-comment-button`,
  `new-comment-popover`, `comment-popover-<id>`.

## Dock panels

- A dock icon toggles its panel. Only one panel is open at a time.
- The panel takes focus when it opens. Escape closes it, stops the event
  there, and returns focus to its icon. The header × does the same.
- A panel keeps its state in stores, not in the open body. Closing
  unmounts the body, and opening it again shows the same data.
- Drag the top edge to resize. The height is remembered.
- Opening a page from a panel (an "Open … page" link, a Review) closes
  the panel: the page would be behind it. `DockPanel` does this for every
  body through the `onOpenPage` it hands down.
- A badge shows a count only. It does not replace the panel's own list.

## Keyboard

- **A list driven from an input uses `useListNav`** (`src/kit/useListNav.ts`):
  ↑/↓ move the cursor, Enter takes the row under it, Escape is the page's
  (clear the filter), the pointer moves the cursor too, and focus stays in
  the input (`aria-activedescendant`). Don't hand-roll another.
- **Every page table and list is keyboard-driven the same way.** Focus the
  table (Tab, or click a row) and ↑/↓ move the cursor (`data-cursor`, drawn
  while the table has focus), Enter opens the cursor row's first link — with
  the column's rows as the opened page's origin — or, when the row has no
  link, runs its first row action. A row that picks a param (`select`) keeps
  its own Enter; keys typed into a control in a cell stay the control's.
- **Shortcuts are commands'.** A shortcut is a command's `ui.shortcut`
  (`.context/commands.md` "Offering a command to a person"), shown on
  its menu item when it has a `ui.menu` place; there's no separate key
  table.
- **Moving through a tab is the browser's keys, not commands.** Cmd+[ and
  Cmd+] are Back and Forward; Alt+↑ and Alt+↓ are Up and Down the list the
  page was opened from (`tabs/navigationKeys.ts`, the global handler in
  `App.tsx`). They don't fire while focus is in a field, a select, a
  contenteditable or Monaco.
- **A page's view is its tab's, and Back follows what you did.** A
  filter change (a range, a search, a toggle) replaces the tab's entry:
  it adds nothing to Back, as a filter bar doesn't. Picking a row (a
  column's `select`) adds an entry: Back un-selects and Forward
  re-selects, as in a mail client. Core decides by the param's role; no
  page chooses. The page doesn't remount for either, so nothing flashes
  Loading and its open sections stay open. A bookmark is of the values
  shown: change a filter and the star goes off; set it back and it's
  on. Closing the active tab shows the most recent other one.
- **The native menu is renderer-driven.** `App.tsx` pushes the menu
  snapshot to `set_native_menu` (built in
  `crates/oxplow-tauri-ipc/src/commands/menu.rs`); macOS shows the
  native bar, off-Mac falls back to the in-window `Menubar`. (There is
  no `isElectron` gate any more — that was dead post-Tauri code.) The
  builder supports **nested submenus** via `MenuItemSnapshot.submenu`;
  dynamic entries (e.g. File ▸ Open Recent ▸ `<project>`, built by
  `buildNativeMenuSnapshots`) use free-form ids like
  `project.openRecent:<path>` that the `menu:command` handler matches by
  prefix rather than going through the menu's items.
- **The macOS application submenu is added in Rust, not the snapshot.**
  `build_menu` prepends a `#[cfg(target_os = "macos")]` "Oxplow"
  submenu of `PredefinedMenuItem`s (About / Hide / Hide Others / Show
  All / Quit) before the renderer's groups, because on macOS the first
  submenu always renders bold under the app name — without it the File
  group lands there and there's no visible Quit. These items are
  OS-standard and state-free, so they stay out of the snapshot (and out
  of the off-Mac in-window `Menubar`).
- **The File menu keeps creating and opening apart** (tsk248): `New
  Project…` (`project.new`) is the only command that initializes a
  folder — it picks a dir, creates `.oxplow/`, and opens it in a new
  window. `Open Project…` / `Open Project in New Window…` / `Open
  Recent` only ever open a folder that already is a project; an
  uninitialized dir errors with a message pointing at New Project…
  rather than silently running first-run setup. The `<Launcher>` mirrors
  the pair (`launcher-new-project` accented, `launcher-open-project`
  secondary). Don't reintroduce an "open initializes it for you" path.
- **The menu bar is File and Edit only** (decided 2026-10-07): the
  offers with a `ui.menu` place (New / Open Project…, Save, Quick Open…
  in File; Find in Edit), beside the native Edit roles (`menuBar.ts`). Every other command is a **search command** — the
  command bus's offers (`commandOffers`: Pull / Push Changes, New Task…
  (⌘⇧N), New Dashboard…, New Stream…, New Thread…, Commit Changes…, New
  Page with Your Agent…). **Pages aren't commands:** the View / Git /
  Tasks "Dashboard" items that only opened a page are gone — the
  launcher lists every page as a page row. Pull / Push run as background
  tasks (failures record an op-error and a "Show details" toast); Commit
  opens the Files page and its commit slideover (its `commit` form).
- **Common muscle memory:** Cmd/Ctrl+S save, Cmd/Ctrl+F find,
  Cmd/Ctrl+P quick open, Cmd/Ctrl+Shift+N new task. Don't
  collide with these.
- **Rows: marks, drags and keys** (every page row viz; Tasks and Backlog are pages).
  Cmd/Ctrl+click toggles a row's mark, Shift+click marks the range
  from the last row clicked, Space toggles the cursor row, Escape clears
  the marks, a plain click clears them. A marked row has a stripe and a
  tint (`[data-marked]`). While rows are marked the marks bar ("N
  selected", the bulk actions, Clear) stands in for the page's buttons.
  Dragging a marked row drags every marked row. A drop's place shows as a
  line above or below the row, or the row or group outlined
  (`[data-drop]`). A row action's `key:` runs it on the cursor row (or
  the marked rows). Marks are per page and kept in the entry's history
  state, so Back keeps them. A dashboard's row tiles (`tile:`) are rows
  too: focusable, right-click and the Menu key open their row actions,
  `key:`s run them, and they drag where a drag action runs, the place
  shown as a bar in the grid gap beside the tile (a drop on the grid's
  background puts it last). Tiles take no marks.
- **A parent row folds its children (`depth` rows).** A row the next row is deeper than shows ▾ / ▸; it folds every deeper row after it away until the next row as shallow as itself. Folds are per page and kept in the entry's history state, like groups'.
- **One launcher is the single discovery surface.** There is exactly
  one way to find pages, commands, files, and content: the launcher
  (`QuickOpenOverlay`), opened by **Cmd+P** (its `file.quickOpen` menu
  command) and the rail **Search…** button. That is its **only**
  shortcut — the old Cmd+K / Cmd+Shift+F aliases were removed (tsk59):
  one door is clearer, and Cmd+P is the established dev quick-open
  reflex. There is no separate command palette or find-in-files overlay.
  Do **not** add a new modal/overlay for discovery, and don't re-add
  alias shortcuts; extend the launcher. See
  `.context/pages-and-tabs.md` → "One Search".
- **Launcher search behavior (the tsk52 pass).** The launcher searches
  the **whole project** for tasks/wiki/notes/comments (cross-stream), but
  scopes file-body hits to the current stream — another worktree's files
  aren't openable here. Implemented as `searchSite(q, null)` +
  a client-side file-hit stream filter in `buildQuickOpenResults`
  (`quickOpenResults.ts`); filename matches already come from the
  current stream's `v_file` (read once, re-read when what it read changes). An **exact-identity match** — a
  task id like `tsk30`, or a page/file/wiki name equal to the query —
  floats to the very top, above the fuzzy pages/commands sections
  (`isExactMatch`). Task ids are searchable because `index_task`
  (`crates/oxplow-app/src/indexer.rs`) folds the id into the FTS body
  (the id is otherwise a non-searchable routing key). The backend body
  search fires only at **≥2 chars** (`MIN_BODY_QUERY_LEN`); a 1-char
  query filters the in-memory pages/commands/files without a round-trip.
  Keyboard: ↑/↓ move the cursor (scrolled into view), **Tab/Shift+Tab
  jump between sections** (`nextSectionIndex` — category headers in the
  start menu, result groups while searching), Home/End → first/last row.
  A section that hit its row cap shows a muted "+N more" footer
  (`QuickOpenBuild.truncated`).
- **The launcher is the main keyboard lever — keep it populated.** Every
  command the bus offers a person that can run now flows into the
  launcher's typed results (`searchableCommands` over the offers), and
  every declared page with a `category` shows in its empty-state start
  menu. When adding a user-visible action, declare it as a command with
  a `ui` over a bespoke button so it stays keyboard-reachable; a new
  *page* needs no command — declaring it under `pages:` with a
  `category` is enough.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test —
  would need to drive:** tabs, primary action buttons, form inputs,
  list items, dock panels. Existing conventions:
  - `dock-tab-<id>` / `dock-panel-<id>` on DockShell rail + content
  - `monaco-host` on the editor container, `data-file-path=<path>`
  - `plan-new-task`, `task-title`, `task-priority`,
    `task-description`, `task-acceptance`, `task-save`,
    `task-save-another`, `task-cancel`
  - `title-bar-search` (the always-visible launcher trigger, in the
    title bar). The launcher
    overlay is `QuickOpenOverlay`; the old `command-palette-input`
    testid is gone (the command palette was removed).
  - `plan-pane` (the keydown-listening wrapper — focus this before
    dispatching keyboard probes, otherwise the listener misses them)
  - `plan-add-points-bar` (now a single ⋯ menu — only "New task" lives
    in it; commit/wait point markers were removed)
  - `files-commit`, `files-commit-message`, `files-commit-submit`
  - Stream rows (`navigator-stream-row-<id>` in the Navigator
    overlay; the stream menu's Add thread opens
    `navigator-new-thread-input`), center tabs
    (`center-tab-<id>`), task rows (`tasks-row-<id>`), terminal tabs
    (`terminal-tab-<id>`), and Navigator threads
    (`navigator-thread-row-<id>`) all open their action menu on
    **right-click** — there are no per-row `*-kebab-<id>` testids any
    more. Right-click the row, then click `menu-item-<id>`.
  - Navigator strip glyphs are `navigator-strip-stream-<id>` /
    `navigator-strip-thread-<id>` (click to switch; a stream glyph also
    opens the panel, and the selected thread's glyph only opens it;
    `title` carries the full name). To open the panel in a test, click `navigator-expand` —
    **hover does not open it** (tsk269). `navigator-collapse` closes it,
    as does a click on the `navigator-overlay` background;
    `navigator-strip-empty` is the strip's dead-space expand target and
    `navigator-new-thread-input` is the inline add-thread field.
  - `menu-item-<item.id>` on every button inside the shared
    `ContextMenu` / `MenuList` — the `MenuItem.id` becomes the
    testid suffix (e.g. `menu-item-task.delete`,
    `menu-item-task.rename`, `menu-item-task.status`,
    `menu-item-task.priority` — rename/status/priority mirror
    the inline click / `s` / `p` shortcuts so keyboard-first users
    don't have to hover)
  - `undo-toast-stack`, `undo-toast-<id>`,
    `undo-toast-action-<id>`, `undo-toast-dismiss-<id>` on the
    Undo toast bottom-stack. The most-recent toast also gets the
    stable aliases `undo-toast`, `undo-toast-undo`, and
    `undo-toast-dismiss` (no id suffix) so probes can target "the
    toast that just appeared" without chasing the random toast id.
  - To open a page in a test, drive the launcher: click `title-bar-search`
    (or open it via Cmd+P), type the page name into the overlay input,
    and Enter / click the result; assert via `page-<kind>` on the body
    (e.g. `page-git-history`, etc.). The old
    `rail-page-<entry-id>` / `rail-pages` testids are gone — the rail no
    longer has a "Pages" section (the Go To panel's bookmarks are the
    pinned set; its rows open them). The `dock-tab-*` testids
    were likewise removed earlier in the IA cleanup.
  - `center-tab-<id>` on CenterTabs tabs (id is `agent` for the
    agent tab, the file's URI for open-file tabs);
    `center-tab-close-<id>` on the × close button. Right-click the tab
    (`fireEvent.contextMenu`) → `menu-item-tab.close-others` /
    `menu-item-tab.close-right` for the universal close actions (plus
    any kind-specific entries)
  - `thread-rail-create-input`, `thread-rail-create-submit` on the
    new-thread creation row; `thread-chip-rename-input-<id>` on the
    inline rename input; `thread-chip-complete-<id>` on the
    hover-card action (also reachable via the chip's right-click menu →
    `menu-item-thread.complete`, or the Menu key / Shift+F10 on a
    focused chip). The Navigator's thread menu has **Mark with…**
    (`menu-item-thread.mark`; submenu `menu-item-thread.mark.<colour|none>`,
    headed by a one-line intent in `context-submenu-header`)
  - `terminal-tab-strip` on the Terminal page's left initials strip;
    `terminal-tab-<id>` on each terminal's glyph button (click to
    activate). Hovering the strip slides out an overlay
    (`terminal-tab-overlay`) with full titles; per-row actions live on
    the overlay row's **right-click** menu — `menu-item-terminal.rename`
    (opens the inline `terminal-tab-rename-input-<id>`; double-clicking
    the overlay title also renames) and `menu-item-terminal.close`
    (kills the shell; disabled when only one terminal remains).
    `terminal-tab-new` on the
    strip's "+" button and `terminal-tab-new-overlay` on the overlay's
    "+ New terminal" button
  These are load-bearing for the browser suite (`tests-e2e/specs/`) —
  don't rename casually; grep the specs before renaming any testid.

## Empty states

- **Every empty page or section uses `EmptyState`**
  (`components/Prompts/EmptyState.tsx`, P6.D2): a title saying what
  would be here, one sentence on how it gets here, and 1–3 prompts the
  person can hand the agent. Each prompt is an Ask — it fills the agent's
  input and never sends. `compact` is the one-line form (rail sections). A loading state isn't an empty one: "Starting the
  agent…" is plain text, and only the empty transcript is an `EmptyState`. Don't offer prompts for what needs the person's own hands
  (typing an AI provider key, approving permissions): say who does it instead. The session
  picker (the New Session page, a thread with no agent session) offers **no**
  prompts — there is no agent here yet to hand one to — only the agent
  choice and Start, which opens the slot and nothing else. Its root
  carries `data-empty-state`, which is how a test tells an `EmptyState`
  from plain copy (`components/Prompts/emptyStates.test.tsx` mounts the
  surfaces that mount cheaply); there is no second empty-copy helper.

## Feedback

- **Show loading state** for any operation >150ms.
- **Show counts** where relevant (e.g., "24 / 500 commits" in the
  history filter).
- **Don't silently drop edits.** Failed operations must surface an
  error near the affected control, not only in the toast area.

## Drag and drop

- **HTML5 DnD needs `dragDropEnabled: false` on the Tauri window.**
  Tauri v2 defaults `dragDropEnabled` to `true`, which registers an
  OS-level drag-drop handler that swallows `dragover`/`drop` before the
  webview DOM sees them — the drag ghost appears but no drop ever fires.
  Every in-app drag here (center-tab reorder, thread/stream rails,
  add-to-agent-context) is DOM drag-and-drop, so the `main` window in
  `apps/desktop/src-tauri/tauri.conf.json` sets `dragDropEnabled: false`.
  Don't re-enable it unless something starts needing Tauri's *native*
  file-drop events (and then reconcile both).
- **Highlight the drop target** (dashed border + accent glow) whenever
  a compatible drag enters it. Clear the highlight on leave/drop.
- **Use a custom MIME type** for internal drags so foreign drags
  (files, text) don't accidentally trigger app drops. **Every internal MIME
  lives in `apps/desktop/src/dragMimes.ts`** — `WORK_ITEM_DRAG_MIME` (list
  rows and Board cards: reorder, restate, multi-select moves),
  `CONTEXT_REF_MIME` ("Add to agent
  context"), `RAIL_SECTION_DRAG_MIME` (RailHud section reorder),
  the row-drag MIME (a page's rows, to where a drag action runs). Add a new
  MIME there rather than overloading an existing one, and rather than
  declaring a `const` next to the component that introduced it: that was
  the old shape, and one task MIME ended up defined twice
  with a unit test whose only job was asserting the copies hadn't drifted
  (tsk271). The module is deliberately import-free so decoders, pure
  helpers, and components can all reach it.
- **Tabs in the center pane are drag-reorderable.**
  CenterTabs reorders **every non-pinned tab freely across the whole
  strip** — there are no per-kind groups. "Pinned" = non-closable (only
  the `agent` tab); it stays at the front and is never a drag source or
  drop target, so nothing lands before it. Reorders persist in the
  thread's tab session (`reorder`, `tabs/session.ts`);
  `App.tsx`'s `handleReorderCenterTabs` reorders the whole strip, not a
  per-kind subset. Clicking a tab in the
  overflow `▾` panel (or any activation of an overflowed tab) promotes it
  to **right after `agent`** via `promoteHiddenIntoStrip` (inserts after
  the leading run of pinned tabs), so it surfaces in the most prominent
  slot. The drop indicator is a **vertical insertion line in the gap**
  (not a box on the target tab): the cursor's half of the hovered tab
  picks before/after, and the drop lands exactly there (`moveToIndex`).
  Pure reorder math lives in `centerTabsReorder.ts` (unit-tested).
- **Right-clicking a center tab** opens its action menu: any kind-specific
  entries (the `CenterTab.contextMenu` slot) followed by the universal **"Close Other Tabs"** /
  **"Close Tabs to the Right"** pair that `CenterTabs` appends for every
  tab. Each close item is disabled when it has no targets; targets are
  computed over the real open tabs in strip order (overflowed-but-open tabs
  still close; `hidden` back/forward stack entries don't). Closes route
  through the host's single-tab `onClose` (file/diff/page dispatch); if the
  sweep takes the active tab, selection falls back to the right-clicked
  anchor. Keyboard parity: tabs are focusable and the Menu key / Shift+F10
  opens the same menu. Pure target-selection math lives in `tabClose.ts`
  (unit-tested); the menu ids are `tab.close-others` / `tab.close-right`
  (→ `menu-item-tab.*` testids).

## Capitalization

- **Title-case for every UI title.** Page titles (`<Page title=…>`),
  tab labels (`label:` in CenterTab arrays), section / card headers
  (`<Section title=…>`, `<Card title=…>`), modal headers, and menu
  items that name a destination (e.g. `New Stream…`) all use
  title case: capitalize the first and last words plus all major
  words (nouns, verbs, adjectives, adverbs, pronouns), and
  lowercase only articles (`a`, `an`, `the`), short prepositions
  (`in`, `on`, `of`, `at`, `to`, `by`, `for`, `with`), and
  coordinating conjunctions (`and`, `but`, `or`, `nor`, `yet`,
  `so`).
  - Right: `Git Dashboard`, `Hook Events`, `Recent Remote
    Branches`, `Ready in This Thread`, `Open in Browser`.
  - Wrong: `Git dashboard`, `Hook events`, `Open in browser`.
- **Sentence-case is OK for inline UI copy** — descriptions,
  hints, button labels that read as commands ("Save", "Cancel"),
  empty-state messages, error toasts. The rule is only for things
  the user reads as a *title*.
- **Mirror the literal across surfaces** — when you change a
  page's title, also update the matching tab label and any
  `deriveDefaultLabel` / `labelByKind` map entry so the renderer
  shows the same string everywhere.

## Numbers

- **Metric values format through `formatMetricValue(value, unit)`** in
  `apps/desktop/src/components/format.ts` — never ad-hoc `toFixed`/hand-rolled
  `k` compaction. It's `Intl.NumberFormat` on the **OS locale** (grouping,
  decimal comma vs point, compact "240.4K" at ≥10k) and unit-aware via the
  spec's `unit` (`%` → one decimal; `ms` → humanized duration). A compacted
  display should carry `formatMetricValueExact(...)` in its hover `title`.
  Deliberately no user-facing locale setting (tsk114): this module is the
  single seam, so adding one later is a one-line change here.

## Empty and error states

- **Every pane has an empty state message** (not just a blank panel).
- **Non-destructive empty states:** "No commits match." rather than
  hiding the filter bar.

## Author badges

- **Runtime auto-filed rows carry a muted `auto` tag** before the
  title (see `AutoAuthorBadge` in `WorkGroupList.tsx`). Human /
  explicit-agent rows render no badge — silence is the dominant path.
  The Work panel header has a `Hide auto` toggle
  (`data-testid="plan-toggle-hide-auto"`) that filters those rows
  out client-side. Preference is local state; no DB persistence
  today.

## Add to agent context

The agent terminal accepts dropped references AND a "Add to agent
context" kebab/menu action; both share one path through
`apps/desktop/src/agent-input-bus.ts` (`insertIntoAgent`) and
`apps/desktop/src/agent-context-ref.ts` (`formatContextMention`).
Inserting fills the agent's input and never sends it. The terminal
pastes what the bus publishes, and xterm turns every line break into
Enter, so `insertIntoAgent` collapses line breaks to spaces (`oneLine`)
for every caller — a manifest's prompt, a selection, a row's text — and
no caller may bypass the bus to write to the terminal.

- **Sources** (anything the user might want to reference): drag rows
  or pills from the Files tree, NotesPane, the WikiActivityBar, the
  Backlinks panel on every Page, the rail HUD recent-files / active
  item / up-next sections, and Code-quality file groups. Set the
  payload with `setContextRefDrag(e, ref)` from
  `apps/desktop/src/agent-context-dnd.ts`. Reuse the same helper and the same
  MIME (`application/x-oxplow-context-ref`) for any new referenceable
  surface — separate from `TASK_DRAG_MIME`, which carries the
  reorder payload.
- **Multi-row task drag** is a separate path. Plan-pane
  `TaskGroupList` drag-start enriches the `TASK_DRAG_MIME`
  payload with `items: [{id,title,status}, …]` so cross-pane drop
  targets can decode resolved refs without their own task
  lookup. The TerminalPane drop handler accepts both
  `CONTEXT_REF_MIME` (single ref) and `TASK_DRAG_MIME`
  (multi-id), iterates the latter, and pastes a space-separated
  chain of mentions in one drop. Helpers:
  `decodeTaskDragRefs` / `dragHasTaskRefs` in
  `apps/desktop/src/agent-context-dnd.ts`.
- **Sink**: `TerminalPane` is the only drop target. It writes through
  `term.paste(text)`, so the same xterm input pipeline handles it.
- **Mention shape** (`formatContextMention`):
  - file → `@<workspace-relative path> ` (Claude reads the file
    automatically on the next prompt).
  - note → `@.oxplow/wiki/<slug>.md `.
  - work item → `[oxplow oxplow:oxplow/work_item?id=<id>&provider=<provider>: "<title>" (<state>)] `
    (plain-text reference; the agent reads it with `read`).
  - any canonical URI → `[oxplow uri <uri>] ` (a drop into a terminal agent). The agent guide says how to read each kind.
  - Always trailing space so the user can keep typing.
- **Right-click parity**: every drag source should also offer "Add to
  agent context" in its right-click menu — keyboard-first users
  shouldn't have to drag. Funnel both paths through the same
  `insertIntoAgent + formatContextMention` calls.
- **Ask = the Assistant.** Every Ask entry opens the Assistant, the
  dock's on-the-spot agent (`components/Assistant/`), about what was asked
  about; nothing goes to a terminal agent. It's on:
  - the page nav bar's ☰ (**Ask the Assistant**, `menu-item-page-ask-assistant`,
    icon `sparkles`, first in the menu), for any page whose tab id is a
    canonical URI (`PageNavigationContext.ask`: `{ ref, streamId }`);
  - a page row's right-click menu (`rowAssistantContext`: the first page
    the row links to, else the row as text);
  - an editor selection and a diff's right-side selection (the selected
    text, with `oxplow:oxplow/file?path=<path>[&rev=<rev>]#lines=<a>-<b>` as
    its source). A diff offers it only while its right side names a file
    (`Diff/diffAsk.ts`, a Monaco context key on the action's precondition);
    a compare with the clipboard has no Ask action rather than one that does
    nothing.
  The catalog's prompts `about` the page's resource (`promptsAbout`) are
  chips above the panel's prompt box; a chip, an empty state's prompt and
  the launcher's last row for any typed text (**Ask the Assistant: <text>**)
  put their question in the box. Nothing in oxplow sends a prompt on its own:
  only Enter does. The prompt box: Enter sends, Shift+Enter adds a line,
  Escape stops a running turn (and is not passed on, or the dock would close);
  a running turn leaves the box read-only, not disabled, so Escape still
  reaches it. States: no model (a link to Settings → AI), suggestions, running,
  a permission card waiting (Allow / Decline), an error. `insertIntoAgent`
  stays for drag-and-drop into a terminal agent.
- **Add to Context** is the collect-then-ask form, and it is universal:
  the ☰ of any page with a canonical URI (`menu-item-page-add-to-context`,
  disabled with "Select a thread first" when none is selected), a
  selection in an editor or a diff's right side, a page row's menu, and the
  floating toolbar on a commentable page's selection (**Add to Context**
  beside Add comment). It puts the page or text in the selected thread's
  basket (`oxplow.context.add`, toast with Undo); the next prompt to any
  agent in the thread carries it. The Context dock panel and page list it.
- **Repair with the Agent** (P7.C3, Settings → Extensions, on a disabled
  contribution with an open repair item) is the same gesture: it fills
  the input with `Repair the extension described in [oxplow uri <item>]
  — read it first.` (`contributionHealth.ts` → `repairWithAgent`). The brief
  itself is the work item's body, not the inserted line — a one-line
  mention is all `insertIntoAgent` can carry.
- **Visual feedback**: drop target shows a dashed accent border +
  centered "Drop to add to agent context" overlay only while a
  payload with our MIME is hovering. Foreign drags (text, OS files)
  must not trigger the overlay.
- **Don't fire `recordUsage`** for these gestures — adding to context
  isn't the same as opening the target; the recents list shouldn't
  reorder just because the user told the agent to look at something.

## A page's controls are in its rail

A page has **one place for its controls: its rail** (`PageRail`). It
has no toolbar, and where a control goes is not the author's to say. The
rail stacks its groups in a fixed order, so nothing moves as values
change: **Filters** (its filter params) and **Selection** (what a row click
picked). A `role: view` param (state the page or the system sets, in the
`#` like a filter) is in neither: the rail never draws it, and a person
never edits it. Page-level actions — About this page (a popover: what the page
is for, in normal text colour, then the same facts Definition's About opens with, less the title and extension — id,
Definition (a link to the running copy drawn as Definition, never the
file to edit; the only way to Definition from a page), origin, what runs — and the tab's URI, and a line when the
source changed since its deploy; a page's description is said only
there, never above its body), Copy, Add to Context, the page's whole-page actions, and
Refresh only for a page that declares `refresh: true` (a page re-runs
on its own when oxplow's data changes; Refresh is for one that reads
outside oxplow) — are in the nav bar (its ☰, unless `prominent`), never
in the rail or above the body (`PageHost` hands them to `Page`).

A param is a **row**, drawn by the kit's `ParamRow`
(`src/kit/ParamControls.tsx`): its label beside its value. At rest the
value is a button ("Any" when it has none; a dot before the label when it
isn't the default). Clicking it, or Enter/Space on it, edits:
- **in place**, the control takes the value's place: text (Enter or blur
  applies), search (applies after a 250 ms pause, at once on Enter),
  choice, branch, the id pickers, a date_range's presets. A select applies
  what is picked and closes. **Escape closes and keeps the value from
  before the edit** (a search puts it back). Focus returns to the value.
- **in a popover** (`kit/Popover.tsx`) when the control doesn't fit a
  320 px row: a date_range's Custom… opens its two dates over the page,
  beside the row. Escape or a click outside closes it; focus returns.
- A **bool** has no edit state: its value is the checkbox.

`ParamList` shows six rows in declared order; the rest wait behind
"More (N)", except a row that is set, which always shows (an active
filter is never hidden). Filters has **Reset** when one is changed. A
page never draws its own param input. One range vocabulary serves every page (`kit/rangePresets.ts`): a preset
key (`1d`…`90d`), `all`, or `a..b` (dates or date-times, either end open) —
a page `date_range`, the metric pages' range and a chart's brushed window. A row shows the value it applied
at once; it never waits for the run.

A param a column `select`s is the page's **selection**, never an input.
A page row whose column `select`s a param is a pick: it reads as
clickable (`ox-clickable`), a click or Enter sets the param, and the row
holding its value is drawn selected (`ox-selected`, `aria-selected`). The
rail's Selection shows the picked row as its facts, labelled as its
columns are, with Clear; with the value but not the row (a reopened page)
it shows the value; with none, "Click a row to select it." A link or
control inside the row keeps its own click (it navigates; it doesn't
pick), and the row's menu stays on right-click and the Menu key.

A column or param with no `label` is headed by its key in words
(`kit/humanize.ts`: `committed_at` → "Committed at"). The page-layout
rules extension authors follow are in the `oxplow-extension` skill
(`references/view-nodes.md`); the loader warns when a page breaks them.
