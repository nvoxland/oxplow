# Usability rules


Things I keep forgetting. Read this before adding any UI.

> **IA redesign — phases 0–7 fully shipped.** Modal `ConfirmDialog`
> and `PromptDialog` chrome was retired in favor of inline patterns;
> per-row actions live on **right-click** menus (the redesign's kebab
> `⋯` buttons were reverted — see "Per-row actions" below); per-stream
> and per-thread settings
> ship as Page tabs (`StreamSettingsPage`, `ThreadSettingsPage`); new-
> stream and new-task flows ship as Page tabs (`NewStreamPage`,
> `NewTaskPage`); snapshot- and commit-detail Slideover wrappers
> (`SnapshotDetailSlideover`, `CommitDetailSlideover`) cover the
> cross-page open path. The rules below describe the redesigned
> target. Phase 7 (density + visual polish) details live in
> `.context/theming.md`'s Density section; the per-phase migration log
> lives in `.context/pages-and-tabs.md`. Plan:
> `/Users/nvoxland/.claude/plans/the-ui-is-very-delightful-badger.md`.

## Forms

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
  a failed run keeps what was typed). Same Enter-submits / Escape-cancels contract;
  the strip is dismissed by the panel's local `pendingPrompt` state.
- **Form-shaped flows that warrant a focused workspace use a page tab
  or a slideover, not a centered modal.** The "+ New" flows ship as
  Page tabs (`NewStreamPage`, `NewTaskPage`, the `Stream/Thread`
  settings pages); cross-page detail openings (snapshot, commit,
  branch rename, file commit) ship as Slideovers. The remaining
  legacy hand-rolled modal chrome inside `PlanPane.tsx`'s
  `NewTaskModal` only backs the edit-double-click flow — do not
  add new modal call sites; route new flows through pages or
  slideovers. The page pattern to copy is
  `apps/desktop/src/pages/SettingsPage.tsx` — full Page tab, no backdrop.
- **A long page has an index, and a link lands on its section**
  (tsk1040). Settings opens with an index of its sections
  (`pages/settingsSections.ts`, `SETTINGS_SECTIONS`); an alert or link that
  means one section calls `goToSettingsSection(id)` before opening the
  page — an open Settings scrolls there, a closed one takes the pending
  request when it mounts. A page that shows state from outside any model
  refreshes on the event that says it changed (Integrations and Data on
  `approvalsChanged`, sent when a person approves a program). Integrations
  also re-reads when an instance's health is recorded (`v_plugin_health`,
  through `useRerunOnChange`): Enable's own `configChanged` can be read
  before the instance has started, and nothing else says it finished
  (tsk1053).
- **A re-read never wipes what the person typed** (tsk1054). A host
  re-reading its data hands a form fresh copies of the same schema and
  saved value, so a form resets its fields on the *content* changing, not
  the object: `SchemaForm` keys on `JSON.stringify` of both, and a row
  holding a draft beside it (`IntegrationRow`'s `config`) does the same.
  Otherwise any event that re-reads — an approval, a config change
  elsewhere, a health record — silently drops unsaved input, and the next
  action sends the old value.
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
  (`page:alerts`): Needs your decision (each proposal's card, Approve /
  Decline), Problems (failed operations; undelivered events and failed
  reactions with Retry / Discard, the same `DeliveryList` as Settings →
  Data), From extensions (firing panel badges). A toast announces each
  new item once (`useAlertToasts`; what's there in the first seconds
  after the app opens counts as seen) and offers only **Review** — never
  Approve: consent is given where the preview is.
- **Async-op failures don't `alert`.** Push a record into
  `opErrorsStore` (`recordOpError({ label, command?, stderr?, stdout?,
  exitCode?, message? })`). A failure is one of the things that need the
  person (below): a toast as it happens ("<label> — Review"),
  the status bar's bell counts it (red), and the **Alerts page** lists
  it under Problems, each row opening to its full output. For ops that
  already have a page focus when they fail (e.g. `runOp` in
  GitDashboardPage), call `onOpenPage(alertsRef())` after recording so
  the person lands where it's shown. The store stays in memory (gone on reload),
  but each record is also reported to the daemon, fire-and-forget, as
  the person-only `ui.report_error` (tsk1072), so the agent reads what
  the person saw in `v_op_error`. A report that fails is logged with
  `logUi`, never recorded as another op error (it would loop).
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
  **Exception: chat prompt boxes.** The ACP thread's prompt box
  (`components/acp/AcpPromptBox.tsx`) follows the chat convention —
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
- **Autofocus the first input** in any inline edit / prompt strip
  when it mounts (and select existing text so the user can replace
  it with a single keystroke).
- **"Save and Another"** for repetitive-entry flows (see the New Work
  Item modal): saves and re-opens the form with the same
  category/priority/parent pre-filled so the user doesn't re-select
  them. Carry this convention forward when New task migrates to
  a page (phase 5e).

## Copy names things as a person knows them

- **No internal ids, codes or placeholders in what a person reads**
  (tsk1044). A thread by its title ("The agent in “Fix the cart”"),
  never `thr1`; a command by its label, its id only on hover; a
  capability by its name ("Version control", `capabilityLabel`), not
  `vcs`; no plan codes ("(P5.E1)") in model docs (`model_docs_carry_no_
  plan_labels`); no `TODO:` scaffold text a person sees (Keep This's
  `my-lenses` gets a real description); counts agree in number ("1
  note"); a status says what happened ("Recorded facts", not "0 rows").

## Agent proposals

- **Approving is the confirmation.** An agent's run that needs a person
  (a person-only setting, a destructive command) waits as a proposal; its
  `ProposalCard` (`components/Proposals/ProposalCard.tsx`) shows who
  proposed it, what it would change (a setting's before → after; a
  composite's commands) and Approve / Decline. Approve runs it as the
  person with no modal; a destructive one is tinted and its Approve is an
  `InlineConfirm` (the per-row destructive rule: one click arms, the
  second runs). A failed decision shows its error on the card, next to
  the buttons. Cards appear in the **Approvals** rail panel and, for a
  setting, on that setting's row in Settings (the compact form). Testids:
  `proposal-<id>`, `proposal-approve-<id>` (a destructive one's:
  `proposal-approve-<id>-trigger` / `-confirm`), `proposal-decline-<id>`,
  `proposal-error-<id>`, `rail-alert-proposals`.
- **A proposal also waits where the conversation is** (P9.A3). A terminal
  thread's Answers strip shows the thread's pending proposals in a
  "Waiting for you (n)" group above its answers — never collapsed, and
  the strip shows for them alone (`thread-proposals`). An ACP transcript
  shows the card under the tool call that made the proposal
  (`proposalOfTool`); once decided, a muted line stays there saying what
  became of it — "Approved by you — it ran.", "Declined by you — nothing
  ran.", "Replaced by a newer proposal." (`acp-proposal-<id>-decided`) —
  so the transcript keeps what the agent asked. An approved one says it
  ran only once its run is recorded (`audit_id`): until then it says
  "Approved by you — running…", since an External command is claimed
  approved before it runs and goes back to waiting if the run fails
  (tsk858). Only the thread's own
  proposals show: a ref the transcript merely quotes shows nothing. The
  Alerts page stays the cross-thread list (its proposal cards).

## Menus a page or row gets from extensions

- **Order is fixed.** A page's nav bar: Ask, then **Commands**
  (extensions' `ui.commands` for the page's ref; hidden when none apply).
  A row's right-click menu: Ask About This, the lens's row actions, then
  a separator and one submenu per provider or extension with its
  commands for the row's ref (`uiCommandMenuItems`). A Board card: Move
  To, then the same extension tail. Each runs through `personCommands`,
  which asks first when the command asks. Testids: `page-nav-commands`,
  `page-nav-command-<id>`, `menu-item-ui-commands-<group>`,
  `menu-item-ui-command-<id>`.

## A custom component is marked

- A `viz: custom` lens (an extension's own web component, sandboxed)
  renders under a small **custom** badge, so the person knows the view
  isn't oxplow's. Its confirmations appear in oxplow (`CommandConfirm`
  below the frame), never inside the frame. When it can't start, a muted
  line says so and the lens's table shows.

## A replaced component is marked, and falls back to oxplow's

- Where an extension's lens stands in for a core component (the Board's
  cards — `Replaceable`, P9.A1), a small **replaced by <extension>**
  badge says whose it is; the page's own chrome stays oxplow's. When the
  replacement can't load, oxplow's own component shows, under a muted
  line saying whose couldn't load and why — never an error instead of
  the page, and never the lens's table. A person switches back for good
  with "Always use oxplow's own <component>" on Settings → Integrations.
  Testids: `replacement-<target>`, `replacement-badge`,
  `replacement-fallback`, `integrations-replacement-<target>`,
  `integrations-replacement-off-<target>`.

## Decorations are additive

- An extension's decorator adds a chip to a page's header (after the
  page's own) or a badge after a lens cell; a page or row is complete
  without them, and a decorator that fails shows nothing. They're labels
  with an extension's name in their tooltip, never actions.

## Destructive actions

- **A button that runs a command asks when the command's spec does**
  (`SpecConfirm`, `apps/desktop/src/components/SpecConfirm.tsx`; tsk898):
  never a second rule in the page — while the spec loads it asks.
- **Per-row destructives use `InlineConfirm`** at
  `apps/desktop/src/components/InlineConfirm.tsx`. First click on the trigger
  swaps to a `[Confirm] [Cancel]` pair in the same horizontal real
  estate. The Confirm button auto-focuses; Escape, blur (outside the
  pair), or Cancel reverts. Examples in tree: Restore button on each
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
- **Closing a dirty file tab** is fire-and-undo: the close completes
  immediately and a toast offers Undo (which restores both the saved
  buffer and the unsaved draft). See `App.tsx` →
  `handleCloseOpenFile`.

## Actions that reach outside oxplow

Some actions do something oxplow can't take back or doesn't own: a
provider's service is written to, a browser sign-in starts, an effect
runs again. The rule is the destructive one's — the person's second
click is the confirmation — plus saying what is about to happen.

- **A retry says what it might repeat** (P9.D4). Settings → Data →
  Delivery lists an effect's failed reactions with why they failed;
  **Retry** is an `InlineConfirm` whose title says a step outside oxplow
  may already have run and that retrying sends it again
  (`reaction-retry-<effect>-<event>-trigger` / `-confirm`).
- **A count a person should see comes before the confirming click**
  (P9.D5). A command's confirmation shows its summary and input, not
  something computed; so **Backfill…** on an approved effect's row first
  reads the plan (`effect.backfill_plan`), says how many events the
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
  provider (a select only when there is more than one), a name, and
  whose it is ("This project's" / "Mine, in every project"). Enter adds,
  Escape clears, and what's wrong with the name is said before Add can
  be pressed. **Remove** is an `InlineConfirm` on a named instance; a
  provider's own instance is turned off, not removed.

## Links open where you ask them to

Any link that resolves to a page — wiki, task, file, directory, commit —
supports the browser conventions: **Cmd/Ctrl-click or middle-click opens
it in a new tab**, plain click navigates in place. The right-click menu
carries the same pair (Open / Open in new tab) so the gesture is
discoverable without knowing the modifier.

The rule that keeps this honest: every page-bearing link resolves
through one `linkTarget(parsed)` and one `navigate(ref, { newTab })`
call (`components/Wiki/MarkdownView.tsx`). Per-kind click branches are
how it drifted before — task links honoured the modifier while file,
directory and commit links silently ignored it (tsk265). A new link kind
gets the behaviour by being added to `linkTarget`, not by growing
another branch.

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
> old `Kebab.tsx` primitive is deleted.)

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
  payload. Some surfaces (FileTree, Plan task rows, the blame gutter)
  instead keep a single parent-owned `ContextMenu` and have each row's
  `onContextMenu` call an existing `onOpenMenu({…, x, y})` opener — pass
  the cursor coords (`new DOMRect(clientX, clientY, 0, 0)` when the
  opener wants a rect).
- **Keyboard parity is required** — right-click is mouse-only, so the
  hook's `onKeyDown` opens the same menu on the **Menu key / Shift+F10**
  for the focused row. Wire it on focusable rows; keyboard-first users
  must never need the mouse. (The Plan pane also covers this with
  `s`/`p`/Enter and `SelectionActionBar`.)
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

## Collapsible page sections

- **A page with stacked sections gets them from the shared primitive**, not
  hand-rolled state — three parts in
  `apps/desktop/src/components/CollapsibleSections.tsx`: `CollapsibleSections`
  (state provider), `CollapsibleSection` (chevron header + hideable body), and
  `SectionCollapseControls` (the Expand all / Collapse all pair). Pure state +
  persistence live in the sibling `sectionCollapse.ts` (unit-tested).
  Adopters: `RecordedMetricsPage`.
- **The page places the controls; they are not pinned above the sections
  (tsk86).** `SectionCollapseControls` reads state off context, so it can render
  anywhere under the provider. Recorded Metrics puts it in the **details rail**
  beside the filters — the rail is the page's control panel. It **self-hides**
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
  (`oxplow.page.sectionsCollapsed.v1`, keyed by `pageKey`), so a newly added
  section appears open with no migration.
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

- **`data-ref-kind` / `data-ref-id` mark a region as a typed "context
  node."** The `(kind,id)` pair uses the same canonical vocabulary as
  tab ids and the `page_ref` graph — the canonical ref kinds of
  [refs.md](./refs.md): `file` / `dir` / `wiki` / `work_item` /
  `commit` / `finding`. Stamp them on the element that *is* that thing
  (e.g. a task row carries `data-ref-kind="work_item"
  data-ref-id="oxplow:tsk42"`). Nesting is meaningful: a file row inside
  a commit card yields the chain `[file, commit, …]`, innermost first. Treat
  these as a first-class seam like `data-testid` — spread
  `contextNodeProps(kind, id)` from
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
  a comment when it lands inside a `data-ref-*` region and outside the
  editor/terminal carve-out, so a single instance safely serves every
  plain-DOM page; a page with no context nodes simply never captures.
  Adding commenting to a new surface is therefore just "stamp
  `data-ref-*` on the regions" — no per-page wiring. The layer captures
  selections, paints existing comments back onto their context node's
  text via the **CSS Custom Highlight API** (no DOM mutation — critical
  for live React lists), and opens a thread popover when a highlight is
  clicked. The quote is re-resolved against the element's `textContent`
  each repaint (debounced to one per frame), so reordering /
  virtualization just re-anchors. The Highlight API is feature-detected;
  where it's absent the comment still works, just without an inline
  highlight. Surfaces opted in so far: task rows (`TaskGroupList`), the
  commit page (`GitCommitPage` meta → `commit`), and commit-graph rows (`CommitGraphTable` → `commit`, under the
  `git-dashboard` root). The **terminal/agent pane** has its own layer
  (`TerminalCommentLayer`, not the app-level one) because it anchors to
  the xterm buffer rather than DOM text — see `.context/terminal.md`.
- **Make the text selectable.** Rows are often `userSelect: none` for
  clean drag — set `userSelect: "text"` on the specific label span you
  want commentable (e.g. the task title) so a quote can be anchored
  without re-enabling selection on the whole row.
- **Draggable rows can't be drag-selected** (a mousedown starts the
  drag), so a floating toolbar never appears on them. For those — task
  rows (`TaskGroupList`) and file-tree rows (`LeftPanel/FileTree`) — add a
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

## Keyboard

- **Shortcuts go through the menu.** Add new shortcuts to
  `commands.ts` and `keybindings.ts` so they appear in the native
  menu and help discoverability.
- **The native menu is renderer-driven.** `App.tsx` pushes the menu
  snapshot to `set_native_menu` (built in
  `crates/oxplow-tauri-ipc/src/commands/menu.rs`); macOS shows the
  native bar, off-Mac falls back to the in-window `Menubar`. (There is
  no `isElectron` gate any more — that was dead post-Tauri code.) The
  builder supports **nested submenus** via `MenuItemSnapshot.submenu`;
  dynamic entries (e.g. File ▸ Open Recent ▸ `<project>`, built by
  `buildNativeMenuSnapshots`) use free-form ids like
  `project.openRecent:<path>` that the `menu:command` handler matches by
  prefix rather than going through the static `CommandId` map.
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
- **The menu bar is File and Edit only** (decided 2026-10-07). Every
  other command is a **search command**: its group (`inMenuBar: false` in
  `commands.ts` — Git: `Commit Changes…` / `Pull Changes` / `Push
  Changes`; Tasks, group id still `plan`: `New Task…` / `New Dashboard…`
  / `New Lens with Your Agent…` / `New Thread…` / `New Stream…`) is
  listed by the launcher under its label, and its keybinding still runs
  (`commandMap` holds every group; `menuBarGroups` picks the bar's for
  the native menu and the in-window `Menubar`). **Pages aren't
  commands:** the View / Git / Tasks "Dashboard" items that only opened
  a page are gone — the launcher lists every page as a page row. Pull /
  Push run as background tasks (failures record an op-error and a "Show
  details" toast); Commit opens the Files page and its commit slideover.
- **Common muscle memory:** Cmd/Ctrl+S save, Cmd/Ctrl+F find,
  Cmd/Ctrl+P quick open, Cmd/Ctrl+Shift+N new task. Don't
  collide with these.
- **Plan pane: single-click selects a task row (keyboard
  cursor); double-click opens the edit modal.** Enter also opens the
  modal for the selected row. Cmd/Ctrl+click toggles the mark set;
  Shift+click ranges from the selected anchor. A plain click clears
  marks and moves the selection. Marked rows render with a yellow
  left-stripe + tint. Dragging any marked row carries every marked
  id in `TASK_DRAG_MIME`'s `itemIds` so drops on the backlog chip, on
  task rows / group headers in `TaskGroupList`, or on the agent
  terminal move all of them at once. Drop targets that handle
  single-item payloads still work — they fall back to `itemId` when
  `itemIds` is absent.
- **Plan pane: a selection-aware action bar appears at the top of the
  work-group region whenever ≥1 row is marked.** Component:
  `apps/desktop/src/components/Plan/SelectionActionBar.tsx`. Buttons mirror the
  marked-set right-click menu — Change status / Change priority /
  Add to agent context / Delete — plus a Clear button. The bar reads
  the existing marked-set state in `PlanPane`; there is no separate
  store. Pure helpers (`shouldShowSelectionActionBar`,
  `summarizeSelection`) are exported for tests.
- **Plan pane: Shift+↑/↓ reorders the selected task within its
  own status section.** Crossing a section boundary is a deliberate
  no-op — to change status, the user drags (which changes status as
  a side effect). Plain ↑/↓ just moves selection; Enter toggles the
  detail pane; `s`/`p` opens the status/priority pickers.
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
  current stream's `listWorkspaceFiles`. An **exact-identity match** — a
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
  enabled menu command in `commands.ts` flows into the launcher's typed
  results automatically (it flattens the same `buildMenuGroups` registry
  via `flattenCommands`), and every page in `computePagesDirectory` shows
  in its empty-state start menu. When adding a user-visible action, prefer
  wiring it as a CommandId over a bespoke button so it stays keyboard-
  reachable; a new *page* needs no CommandId — adding it to
  `computePagesDirectory` (with a `category`) is enough.

## Test-driveability

- **Add a `data-testid` to every new seam a user — or a test —
  would need to drive:** tabs, primary action buttons, form inputs,
  list items, dock panels. Existing conventions:
  - `dock-tab-<id>` / `dock-panel-<id>` on DockShell rail + content
  - `file-tree-entry-<path>` on FileTree nodes (plus `data-kind` and,
    for dirs, `data-expanded`)
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
    (e.g. `page-git-history`, `page-local-history`, etc.). The old
    `rail-page-<entry-id>` / `rail-pages` testids are gone — the rail no
    longer has a "Pages" section (the Go To panel's bookmarks are the
    pinned set; its rows open them). The `dock-tab-*` testids
    were likewise removed earlier in the IA cleanup.
  - `center-tab-<id>` on CenterTabs tabs (id is `agent` for the
    agent tab, `file:<path>` for open-file tabs);
    `center-tab-close-<id>` on the × close button. Right-click the tab
    (`fireEvent.contextMenu`) → `menu-item-tab.close-others` /
    `menu-item-tab.close-right` for the universal close actions (plus
    any kind-specific entries)
  - `thread-rail-create-input`, `thread-rail-create-submit` on the
    new-thread creation row; `thread-chip-rename-input-<id>` on the
    inline rename input; `thread-chip-promote-<id>` and
    `thread-chip-complete-<id>` on the hover-card actions (also
    reachable via the chip's right-click menu →
    `menu-item-thread.promote` / `menu-item-thread.complete`, or the
    Menu key / Shift+F10 on a focused chip — keyboard-first users should
    never have to hover to promote a thread)
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
  input and never sends. `compact` is the one-line form (rail sections,
  the ACP transcript). A loading state isn't an empty one: "Starting the
  agent…" is plain text, and only the empty transcript is an `EmptyState`. Don't offer prompts for what an agent can't do
  (AI providers, keys, consent): say who does it instead. Its root
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
  lives in `apps/desktop/src/dragMimes.ts`** — `TASK_DRAG_MIME` (task
  reorder / multi-select moves), `CONTEXT_REF_MIME` ("Add to agent
  context"), `RAIL_SECTION_DRAG_MIME` (RailHud section reorder). Add a new
  MIME there rather than overloading an existing one, and rather than
  declaring a `const` next to the component that introduced it: that was
  the old shape, and `application/x-oxplow-task` ended up defined twice
  with a unit test whose only job was asserting the copies hadn't drifted
  (tsk271). The module is deliberately import-free so decoders, pure
  helpers, and components can all reach it.
- **Tabs in the three tabbed sections (left dock rail, center pane, bottom
  dock rail) are drag-reorderable.** DockShell rail tabs persist their order
  in the dock's `localStorage` entry (`oxplow.layout.v1.dock.<key>.order`).
  CenterTabs reorders **every non-pinned tab freely across the whole
  strip** — there are no per-kind groups. "Pinned" = non-closable (only
  the `agent` tab); it stays at the front and is never a drag source or
  drop target, so nothing lands before it. Reorders persist by rewriting
  the unified `threadPageTabs` order (the strip renders
  `[agent, ...threadPageTabs]`); `App.tsx`'s `handleReorderCenterTabs`
  reorders that whole list, not a per-kind subset. Clicking a tab in the
  overflow `▾` panel (or any activation of an overflowed tab) promotes it
  to **right after `agent`** via `promoteHiddenIntoStrip` (inserts after
  the leading run of pinned tabs), so it surfaces in the most prominent
  slot. The drop indicator is a **vertical insertion line in the gap**
  (not a box on the target tab): the cursor's half of the hovered tab
  picks before/after, and the drop lands exactly there (`moveToIndex`).
  Pure reorder math lives in `centerTabsReorder.ts` (unit-tested).
- **Right-clicking a center tab** opens its action menu: any kind-specific
  entries (the `CenterTab.contextMenu` slot — e.g. external-url's "Open in
  Browser") followed by the universal **"Close Other Tabs"** /
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
  - task → `[oxplow task <id>: "<title>" (<status>)] `
    (plain-text reference; agent can fetch via
    `oxplow__get_task`).
  - any canonical ref → `[oxplow ref <ref>] ` — **Ask About This**
    (below). The agent guide says how to read each kind.
  - Always trailing space so the user can keep typing.
- **Right-click parity**: every drag source should also offer "Add to
  agent context" in its right-click menu — keyboard-first users
  shouldn't have to drag. Funnel both paths through the same
  `insertIntoAgent + formatContextMention` calls.
- **Ask About This** (P6.D1) is the conversation-first form of the
  same gesture: it puts `[oxplow ref <ref>] ` in the agent's input for
  the person to finish the question, and never sends. It's on:
  - the page nav bar (`page-nav-ask`), for any page whose tab id is a
    canonical ref (`PageNavigationContext.ask`: `{ ref, streamId }`);
  - a lens row's right-click menu (`rowAsk`: the first ref the row links
    to, else the row as a lens mention);
  - an editor selection and a diff's right-side selection
    (`askAboutSelection`: `file:<path>[@rev]#L<a>-<b>`). A diff offers it
    only while its right side names a file (`Diff/diffAsk.ts`, a Monaco
    context key on the action's precondition); a compare with the
    clipboard has no Ask action rather than one that does nothing.
  The nav bar's Ask is a menu (`components/Prompts/AskMenu.tsx`): Ask
  About This, then the catalog's prompts `about` the page's ref kind
  (`SuggestedPrompts`), each inserted as `[oxplow ref <ref>] <question>`.
  The launcher's last row for any typed text is **Ask the Agent: <text>**,
  which inserts the text the same way. Nothing in oxplow sends agent
  input on its own; these only fill the draft.
- **Repair with the Agent** (P7.C3, Settings → Extensions, on a disabled
  contribution with an open repair item) is the same gesture: it fills
  the input with `Repair the extension described in [oxplow ref <item>]
  — read it first.` (`pluginHealth.ts` → `repairWithAgent`). The brief
  itself is the work item's body, not the inserted line — a one-line
  mention is all `insertIntoAgent` can carry.
- **Visual feedback**: drop target shows a dashed accent border +
  centered "Drop to add to agent context" overlay only while a
  payload with our MIME is hovering. Foreign drags (text, OS files)
  must not trigger the overlay.
- **Don't fire `recordUsage`** for these gestures — adding to context
  isn't the same as opening the target; the recents list shouldn't
  reorder just because the user told the agent to look at something.
