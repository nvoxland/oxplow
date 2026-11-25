# Extensions: Custom components

Custom components (the sandboxed frame, protocol, and testing a component). Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## Shipped components

- **The Work board: oxplow-bundled's built component** (`pages/work.yaml`,
  `custom_components: board`, source `extensions/oxplow-bundled/src/board/`,
  built by bundled's `package.json` with `bun build`, no dependencies). The
  worked example of a custom page that **reuses core where it fits and draws
  the rest itself**. Core's: the nav bar (`setTitle`, `setActions`: New
  Task…), the confirmation a delete asks for, the record and line of an
  `invoke` that fails, the tab's `#` with Back and Forward, the page's
  loaders as its data (the shared `task-rows` query), the kit's sheet
  (`.ox-toolbar`, `.ox-select`, `.ox-input`, `.ox-board`, `.ox-card`, the
  drag classes and `.ox-menu`) and the theme's tokens. Its own: the
  toolbar, the columns, drag and drop, and a card's menu. **A custom page
  owns its controls through `role: view` params** (`show`, `priority`,
  `q`): they ride the tab's `#` like filters, the rail never draws them,
  and the component sets them with `setViewParams` and hears Back through
  `onViewParams`. With no `filter` param, `rail:`, sections or see-also, the
  page draws no rail and the frame has the whole width. Its pure half
  (`model.ts`) is tested from the desktop suite (`viz/boardModel.test.ts`),
  the frame by a `ui:` fixture. What the library doesn't give it: icons (a
  card has no state glyph; its column says the state) and the host's own
  context menu (it draws `.ox-menu`).
- **Foundation's only custom component** is the hand-written
  `integrations`.

A component can send an inline page to core with `showView`. Core draws
its loaders and nodes below the frame, and **Save as Page** writes that
specification to an extension. This is the same page kit used for views
opened from an agent answer (see [Showing a page](./pages-and-tabs.md#showing-a-page)).

## Custom components

`custom_components:` (**stable**; `extensions/custom_components.rs`) are
web bundles that a page shows in a sandboxed frame. A page uses one
through a `component` node (`view: { component: { id, props } }`, alone
as the whole page or inside a layout) or a `viz: custom` node
(`custom: { component, props }`). **The sandbox bounds what it
reaches.** The frame has an opaque origin (the iframe's
`sandbox="allow-scripts allow-forms"` *and* the daemon's `sandbox
allow-scripts allow-forms` CSP directive), no daemon token and no way to
send data. It has no fetch/XHR/WebSocket (`connect-src 'none'`), no form
submission (`form-action 'none'`: a component's `<form>` is its own, its
script handles `submit` and runs a command with `invoke`) and no
storage. The one way out is navigating itself. The page's `frame-src`
(`http://127.0.0.1:*`, `http://localhost:*` — each loopback name the
daemon serves a bundle to; not `[::1]`, which no CSP host source can
name; and `blob:`, the `media` viz's PDF frame, bytes the page itself
holds) bounds that to this machine. That is Tauri's CSP in the app, and a
one-directive meta CSP in `index.html` for a plain browser, where
whatever serves `dist/` sends no header (`viz/frameBound.test.ts` keeps
them one value; only `frame-src`, since a fuller policy would bound what
the page connects to, and a daemon over a tunnel is not on loopback).
The host also ends the component on its second `load`. So a component
reaches only what its page feeds it, its own read-only SQL (`sql:
read`), the commands it may invoke (`commands`) and the action points it
may run (`action_points`).

**A component that acts runs under its extension's permissions.**
`invoke` runs a command with the viewer's rights, so the commands a
component declares are the only ones it can run. A command outside its
extension's namespace needs `commands:<pattern>` in the manifest
(extensions-permissions.md "Permissions"; the loader refuses the extension otherwise). A person
approved those permissions when the extension was deployed. An `invoke`
of a command that the loaded bundle does not declare is `Invalid`. An
`invoke` from a version that is not the current one is `Denied` (the
frame must reload). Every `invoke` runs through the person's runner
(usability.md "A person's command that fails"): any failure but a field
error is recorded once and shown under the frame
(`custom-component-refused`, an `InlineError` anchored `component/<ext>/<id>`),
whatever the component's own code does — the frame still gets its
rejection, and need only draw a field error. A component that declares no
commands only shows and reads.

**What runs is what was deployed.** Before it shows a frame the host
loads the component's bundle (`load_component`, from the deployed
catalog, the same whatever the stream) into `Services::component_bundles`
(`component_bundles.rs`): the bundle's folder in its deployment and its
file names, keyed by its **version** — a hash over those files, its
commands and its `sql` access (`bundle_version`). No bytes are held. The
daemon serves the frame (`/components/v/<version>/…`) by reading the
named file from that folder, which never changes. The frame's `invoke`
names the version it loaded, and runs only when that version is the
current one and declares the command. So an edit to the source changes
nothing that runs until it is deployed. The folder is read as served —
the extension's path joined with the manifest's `bundle` — so a spelling
that differs from the disk's only in case still covers every file. Every
version loaded in the process stays servable while its deployment is in
the store (a folder and a list of names each, so nothing is dropped).
Once the deployment is pruned, its files 404 and the frame reloads.

**A hot update reaches a mounted frame.** Activation stages every
enabled extension's bundles into the generation it activates
(`PreparedComponentBundles`), each stamped with that generation. A frame
whose snapshot belongs to a generation that is no longer active has its
`querySql` and `invoke` refused. The host loads the bundle again on every
`extensionsChanged` (sent after an activation completes,
`CustomComponentViz`): the same version keeps the frame it has, a new
one mounts a new frame, and the old version's `invoke` is refused.

**Bundled and project components share one runtime.** A bundled
extension runs from its shipped deployment on disk (`Sources and
deployments`), so its component's bundle is staged, hashed, served at
`/components/v/<version>/` under the same CSP and — when it declares
`commands` — held to its extension's permissions exactly like a project's.
What still differs by origin, none of it about components: a bundled
extension with no deployment of its own runs what ships
(`shipped_permissions`), the `oxplow` command namespace is reserved
for bundled extensions, and a bundled extension's name wins a clash with
a project one's.

```yaml
custom_components:
  - id: burndown                       # [a-z0-9-]+
    title: Burndown
    description: Open tasks against the days left in a sprint.   # optional: one sentence (v_custom_component.description; none is made up)
    bundle: components/burndown        # default components/<id>; holds index.html
    commands: [oxtask.task.move]
    sql: read                          # optional: its own read-only SQL (querySql); needs sql.read in permissions
    size: content                      # optional: content (default) | fill
```

`size` says how tall the frame is. `content`: as tall as the content the
component draws (the client library reports its height), no border, the
page scrolls. `fill`: the frame takes the page's height and the component
scrolls itself (the Work board declares it). A run fills each `viz: custom`
and each `component:` node with its component's `sizing` from the
declaration (`size_components` in `custom_components.rs`), and a `fill`
custom viz fills the page. A lone `content` custom viz does not.
`v_custom_component.size` shows it.
```yaml
# pages/burn.yaml
title: Burndown
loaders:
  burndown:
    sql: SELECT day, remaining FROM v_x_burndown WHERE stream_id = :stream_id
params: [{ name: stream_id }]
view:
  viz: custom
  from: burndown
  custom: { component: burndown, props: { color: accent } }
```

**A component reads what its page feeds it.** A component has no
`assets`: that key is refused as an unknown field. The page's loaders are
its data (`init.data`, `update`). For its own queries it declares `sql:
read` (`querySql`). Load checks, each at its line: the id, a duplicate,
`bundle` a relative folder without `..` that holds `index.html`, no
symlink anywhere in it (`stat_bundle`; a bundle has no size limit, see
Deploy) and each command a command name. A page that names a component
its extension does not declare is refused (`component_problem`: "`x`
isn't one of this extension's `custom_components`"; `<namespace>/<id>` for
another extension's component is that extension's to have). A `viz:
custom` node needs `custom.component` and a `from` loader (its rows are
what the component shows and what an agent reads). An agent reads a
`viz: custom` node as its table, prefixed ``(custom component
`<namespace>/<id>`; its table rendering)``, and a `component` node as
``(custom component `<id>`: it shows the page's loaders)``.
`check_extension` checks that the declared commands are registered, and
warns when a custom viz also fills a kit role block (`chart`, `tree`,
`timeline`, `steps`, `sections`, `hunks`, `code`, `diff`, `editor`, `embed`,
`terminal`) — the kit may already render it. That is the honest extent of
a "this reimplements the kit" lint.

**A component runs an action point** it declares in `action_points:
[<point id>]` (any extension's point; each id is checked at load with
`is_action_point_id`, `component.invalid_action_point`). The bundle records
its points beside its commands, in its version hash, so what a person
approved is what runs. `invoke_component_point { id, point, context,
command?, confirmed, version }` (`viz_actions::invoke_component_point`)
checks the bundle declares `point`, asks the point's offer as the viewer
(`action_point_offer`, which applies `ui.accepts`), and runs the command
the offer picks with the offer's bound input — `command` if given and
offered, else the only one. Nothing offered is `Invalid` ("nothing answers
`<point>` for this context"), several with none named is `Invalid` listing
them. It runs exactly as `invoke_component_command` does (actor
`Component { on_behalf_of: Human }`, the host's confirm path, audit) but
needs no `commands:<pattern>` permission and no `commands:` entry: the
store joining the point is its consent. The frame calls
`invokePoint(point, context, command?)`.

**The bridged calls** (`viz_actions.rs`; the frame reaches them only
through the host). Each names the component by `id`, `<namespace>/<component
id>` — the component itself, whatever shows it — and the bundle `version`
its frame was loaded at (`load_component { id }`). A version that is not
the current one is `Denied`. `invoke_component_command { id, command,
input, confirmed, version }` runs `command` when it is one of the
component's `commands` at that version, as **`Actor::Component {
component: id, on_behalf_of: Human }`** — the component acting for the
person, so every policy applies as if they ran it. It is audited as kind
`component` with the component's contribution ref as source
(`command.executed@6`). The input is literal (no placeholders). A command
that asks comes back `NEEDS_CONFIRMATION` and the **host** asks, never
the frame. A component acts for a person, so it never proposes.
`run_component_sql` and `component_form` are the same kind of call. All
are UI RPCs (`ui` in surface parity).

**The host** (`viz/CustomComponentViz.tsx` for a `viz: custom` node,
`pages/PageComponent.tsx` for a `component` node, both over
`viz/ComponentFrame.tsx` and `viz/componentBridge.ts`):
`<iframe sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer">`
at `componentBundleUrl(base, version)` (the daemon's
`/components/v/<version>/`, once `load_component` answered; a bundle that
cannot be loaded shows the table and why; with no daemon base —
`remoteBaseUrl()` null — the page shows its table), with no badge (whose
it is shows in the page's About). On the frame's first `load` the host posts `init { protocol, run,
data, params, props, page, view, tokens }` (`initMessage`, the one
builder) with one end of a `MessageChannel` (to `"*"`: the frame's origin
is opaque; the port goes to that frame alone) and listens on the other
end only. `data` holds each loader's `{ columns, rows, truncated, error? }`
by name (`pageData`), `params` what they ran with, `props` the node's
`props` (or a viz's `custom.props`). `run` is a `viz: custom` node's own
rows as a `VizRun`, and null for a `component` node. `page { streamId,
threadId }` is set when the component is a page's `component` node, else
null. `view { viewParams, historyState }` is its host's view, or null. A
change follows as `{ type: update, run, data, params, changed }`, with
`changed` the loaders whose rows differ (posted only when something
did).

Frame messages (`parseFrameMessage`): `ready`; `{ id, method: invoke,
command, input }`; `{ id, method: invokePoint, point, context, command? }`
(`invoke_component_point`); `{ id, method: navigate, uri }` (an oxplow page only:
`componentMayOpen` refuses an external-url URI with `INVALID`, since an
outside tab would carry whatever the frame put in its URL out); `{ id,
method: querySql, sql, params }` (positional SqlCells;
`run_component_sql`: the component must declare `sql: read` and its
extension hold `sql.read`; it runs as the component's `Mode::Pure`
run under its extension's grant, `sql.read` decided and recorded,
read-only over the published datasets, 1,000 rows by default); `{ id,
method: openForm, command, input }` (the host shows a declared command's
form — `component_form` — in a slideover and runs the submission through
`invoke_component_command`; closing it answers `CANCELLED`); `{ id,
method: openForm, form: { title, schema, values? } }` (a form of the
component's own: the host draws `schema`, a JSON Schema where a
`writeOnly` string is masked, with the same `SchemaForm` and answers the
values submitted, running nothing); `{ id, method: signIn, instance, name
}` and `{ id, method: cancelSignIn, instance, name }` (the host runs the
whole OAuth redirect flow for a provider instance's credential,
`viz/hostSignIn.ts`: the shell listens on the credential's registered
port, the system browser opens the service's page, each redirect goes to
the core; it answers `{ state: signed_in }`, `{ state: failed, error }` or
`{ state: cancelled }`). Each step is a command the component must declare
(`oxplow.provider.begin_sign_in`, `complete_sign_in`, `cancel_sign_in`),
run as the component. Without the desktop app it answers `INVALID`. A
newer sign-in of the credential, a `cancelSignIn` or closing the frame
ends the one under way.

A component that is a whole page also sends `{ id, method: setTitle, title
}` and `{ id, method: setActions, actions: [{ id, label, primary?,
prominent?, submenu?, icon?, disabled? }] }` (the page's title and its
nav-bar actions, data core draws: the primary and prominent ones as
buttons, the rest in the bar's ☰; a press posts `{ type: action, id }` to
the frame), and `{ id, method: showView, spec }` (an inline page's YAML,
read-only, `parse_inline_page`, that the page opens as a run of its own
— `useInlineView`, held open while it shows, so it stays live — and draws
under the frame with `PageTree`, with Save as Page when it is one viz
over one `sql` loader). `showView` answers its first loader's rows, for
the frame's own pickers, or `INVALID` — the open's refusal, or a failed
loader with `field: /loaders/<loader>/sql`. A null spec clears it. While a
view shows, the frame sits at its content's height: the library posts
`{ type: size, height }` (a message, not a request, from a
`ResizeObserver` on its root; a component leaves `html` and `body` at
`height: auto`). `component.reveal({ top, bottom })` (protocol 14; px from
the component's top) asks the host to scroll what holds the frame so that
part is in view: a sandboxed frame cannot scroll its page
(`viz/revealInFrame.ts`). `setTitle`, `setActions` and `showView` are a
whole-page component's. `openForm`, `signIn` and `cancelSignIn` are any
host's that draws a slideover — a whole page and a component inside a
page's layout, a settings section included (`viz/frameForms.tsx`,
`useFrameForms`). A host with none answers `INVALID`. And — each kind of
page state by the name of its scope — `{ id, method: setViewParams,
params }` (string values, null drops one: the `#` of its tab entry,
replaced; a component has no selection role, so it never pushes, and
going somewhere is `navigate`) and `{ id, method: setHistoryState, patch
}` (this step of its tab). The host posts `{ type: viewParams, params }`
and `{ type: historyState, state }` when the entry's change (Back,
Forward, an echo; `createBridgeHost` sends only what the frame has not
seen). `init.view` carries both.

### Testing a component

A page fixture's step `{ ui: [..], node? }` runs the page's custom
component — its `node:`, else its first `component` / `viz: custom` node,
else the first in a page it embeds; `node: <embed>/<node>` names one inside
an embed, a slot's mount or a replacement (`component_at`; on each re-run
the harness is fed that embedded run again, `embedded_at`) — **in the app's own webview, against the test's oxplow, with no app
around it** (`component_harness.rs`, `PageAt::ui`):

- **The server** (loopback, per step) serves the harness's page (`/harness/`,
  `crates/oxplow-sdk/assets/harness-host.js`), the bundle exactly as the
  daemon serves it (`oxplow_app::component_http`: the same files, CSP and
  library) with the agent (`assets/harness-agent.js`, served beside the
  library so the CSP allows it) put first in its `index.html`.
- **The host** (`apps/desktop/src/viz/harnessHost.ts`, built by `bun run
  build:harness`; `harnessBuild.test.ts` fails when the committed build is
  stale or pulls in React; the comparison is byte for byte, so CI runs the
  Bun of `.bun-version` and a Bun upgrade rebuilds the file and bumps that
  version together) mounts the frame with the app's own
  `createBridgeHost` / `initMessage` / `pageData`. What the core answers —
  `invoke` (`invoke_component_command`, as the person), `invokePoint`
  (`invoke_component_point`; recorded in `invoked` with `point` and
  `context`), `querySql`
  (`run_component_sql`), `openForm` / `askForm`, `showView` — runs on the
  test's oxplow; what only the shell answers (`setTitle`, `setActions`,
  `setViewParams`, `setHistoryState`, `navigate`, a confirmation) is
  recorded, in the order made (one queue of posts, which the next op waits
  for). `showView` runs one at a time in the order asked and is recorded
  once the core has answered: `shown` is the spec when the core drew it,
  `null` when it refused it or the component cleared it. The server drives
  it one op at a time (`/harness/next`): a DOM step for the agent, a nav-bar
  press, a view change, new rows.
- **Start-up wait**: the test waits up to `START_WAIT` (60 s) for the page's
  `drawing` word — the window is a process start plus a page load, which a
  loaded machine stretches far past a few seconds (a fixed 10 s made
  `ui:` steps flake under load). A window that exits ends the wait at once
  and the error names the exit; at the bound the error says what never came
  and for how long (`first_signal`).
- **The window** is the shell's `oxplow component-harness <url>`: one
  window, no Dock icon, never focused, no project, daemon, menu or IPC; it
  quits at `/harness/done`. Tests run while the person works, so it is
  never seen: on macOS it is shown (WebKit draws no frames for a hidden
  window, or for one off every display, and macOS moves a titled window
  placed off-screen onto a display left of or above the main one) but
  borderless, fully transparent and click-through (`unseen` in the
  shell's `main.rs`); elsewhere it goes to -20000, -20000. Found as `OXPLOW_SHELL`, else an
  `oxplow` beside the running program or up to two folders above it. With
  none, the `ui` steps aren't run, nor the steps after them (they read
  what the `ui` steps did): the report lists it under `skipped` and its
  case is **skipped** (`CaseOutcome::Skipped`, a `<skipped>` in the JUnit
  form) — never a pass.
  Likewise when the window draws no frames: WebKit draws none while the
  display is locked or asleep, so a component that lays itself out never
  draws. The page asks first (`drawsFrames`: a frame within 2 s) and
  records `drawing`; `false` leaves the steps, and those after them, not
  run and the case skipped, never failed (`Harness::drawing`). CI builds the app and runs the tests
  under `xvfb-run`.
- **Steps**: `click: <sel>`, `fill: { <sel>: text }`, `key: { on, key }`
  (`Mod+Enter`; a plain `Enter` in a form's `<input>` that nothing stopped
  submits the form, as a browser's implicit submission does),
  `drag: { from, to }` (synthetic DOM events, not a
  person's: focus and keyboard details can differ); `press: <action id>`
  (the nav bar), `view_params: {..}` (Back/Forward; `null` drops one). The view
  params the frame set (`setViewParams`) and the ones a step sent are kept,
  merged, and when they change the page runs again with them over the
  step's `params`, and the frame hears the rows, as the app's tab does with
  its `#`; `given: [..]` (the
  page runs again and the frame hears its rows, as after any command it
  ran); `confirm: false` (the next confirmation's answer; default yes),
  `form: { submit: {..} }` (the next form's values; without, it's closed);
  `expect: { visible, count: { <sel>: n }, text: { <sel>: text | {
  contains } }, title, actions: [ids], view_params, invoked: [{ command,
  input, ok, result | refused }], navigated, confirmed, forms, shown, revealed: [{ top, bottom }] }`,
  compared by the one rule and waited for (5 s). A finding names
  `steps[i]: ui[j]` and carries the frame's HTML (4 kB) and the errors
  the agent caught.
- **What it can't see**: how the app draws the frame, the nav bar or the
  slideover (the generic `tests-e2e/specs/pages/component.spec.ts`), real
  input events, or how it looks.

**A page's rail.** A page file may have a `rail:` node tree beside its
`view:`. The page host draws it in the page's rail, and a row it picks
writes the page's view by the rail's rule (`pageViewChange`: a selection
pushes a history step, a filter replaces), which a whole-page component
hears through `onViewParams`. A page's rail stacked above its body (a
narrow window) is capped and scrolls, so the body keeps the height. A
custom page whose controls are its own declares them `role: view`
(the Work board): no rail is drawn, and the frame takes the width.

**Whose view a component has** (protocol 9, `viz/hostedView.ts`
`useHostedView`): its host's, where the host shares one, and only what
the host's contract names.

- A page that **is** the component (`component` as the whole view) has
  the whole entry: all its params (`patchViewParams`) and its history
  state.
- A **replacement** has the host page's view params that its target's
  contract lists (`UiReplacement.view`, `v_replaceable.view`: the Work
  page's `show`, `priority`, `q` for the board). These are the
  ones the core part it replaces reads, so the two leave the same
  address. Another key answers `INVALID` naming the contract.
- A **`viz: custom` node** on a page has the page's non-resource params
  (`pageViewKeys`: its filters and picks), set by the rail's rule
  (`pageViewChange`).

A hosted component's history state is its own object in the entry,
`component:<target>` or `component:page`, so it never overwrites its
page's (a draft, a pick). Every other host — a dashboard tile, a slot,
the rail HUD — gives none: `init.view` is null, `setViewParams` and
`setHistoryState` answer `INVALID`, and the component keeps its filters
itself. Any component, page or not, has **page memory**: `{ id, method:
pageMemoryGet, key }` and `{ id, method: pageMemorySet, key, value }`
(null forgets), scope `component:<namespace>/<id>` in `ui_preference` — for
the person, in every tab of it. A value over 64 KB or a component over
1 MB is refused `INVALID` (`componentMemoryProblem`, before the write).
A component keeps no view state of its own: `check` warns
(`bundle_warnings`) at a bundle script that uses `localStorage` or
`sessionStorage`.

Replies are `{ id, ok, result | error: { code, message, field? } }`
(`field`: the input an `INVALID` refusal names, so a component's form
marks it). A re-run sends `update` — only when its params or result
differ from what the frame last got (`createBridgeHost` takes the run
`init` carried). A second `ready` is ignored. The frame is keyed by its
bundle URL (`ComponentFrame`): a stream switch mounts a new frame whose
first `load` is its own, and a failure is remembered per URL. An invoke
that asks shows the host's `CommandConfirm` with what it asks about
(`details`, the bridge's `confirm(command, details)`); confirming re-runs it
confirmed, declining answers `CANCELLED`. No `ready` within 3 s, or a
second `load` (the frame navigated itself), tears it down and shows the
table. `ready` records `usage { kind: "custom_component", key: <namespace>/<id>
}` — the evidence a kind needs to be promoted. `tokens` are the root's CSS
custom properties (`tokensFromStyle`). There is no gesture check on
invoke: the frame's clicks do not reliably activate the host across
webviews, and the real bounds are the declared list, the person's policy
and the host's confirmation.

**The client library.** A bundle does not speak the protocol by
hand. The daemon serves `oxplow-component.js` (and its `.d.ts`) at
`/component-lib/<file>` — embedded in the daemon
(`crates/oxplow-app/assets/component/`, `component_http::library`), served
like a bundle (loopback `Host` only, ungated, outside CORS, `nosniff`),
and named in every bundle's CSP `script-src` beside the bundle's own
folder (`bundle_csp(source, lib)`). There is no `'self'` in that CSP:
even in a sandboxed frame it matches the daemon's whole origin — the
response URL's, checked in Chromium and WebKit — so it would let a
component load another bundle's code. The check lint reports a page path
that leaves the bundle (`..` past its folder, an absolute path other than
`/component-lib/`, a `data:` script or sheet). The library is a **classic
script that defines the global `oxplow`**, not a module: module scripts
are fetched with CORS, which an opaque origin never passes and nothing
served to a frame allows — the same reason a bundle's own scripts are
classic. `oxplow.connect()` waits for `init`, answers `ready`, and
resolves with `{ data, params, run (the latest of each), page, props,
tokens, protocol, onUpdate(fn({ run, data, params, changed })) →
unsubscribe, invoke(command, input), invokePoint(point, context, command?),
navigate(uri, { newTab, replace }),
querySql, renderMarkdown(element, text, { inline? }), openForm (a
command's, or `{ title, schema, values }`),
signIn, cancelSignIn, setTitle, setActions, showView, onAction,
viewParams, setViewParams, onViewParams, historyState, setHistoryState,
onHistoryState, pageMemory, applyTheme(doc?) }` (the `.d.ts` beside it
is the reference; `viewParams` / `historyState` are null where the host
gives no view). A failed request rejects with the host's `{ code,
message }`. `init` carries `protocol` (`BRIDGE_PROTOCOL` in
`componentBridge.ts`, `PROTOCOL` in the library — one number, bumped when
a message's shape changes; now 14), and `connect` rejects a host that
speaks another. One app is one library version. `componentClient.test.ts`
drives the served file against the real `createBridgeHost` and the real
`init` (`initMessage`, which `CustomComponentViz` posts), so the two
cannot drift.

**Markdown in a frame** (`renderMarkdown`, protocol 13). A component
sends the text (`markdown { text, inline }`); the host answers the tree
to draw (`markdownTree`, `components/Markdown/markdownTree.ts`): the
viewer's parse (remark + GFM), its wikilinks and link rules
(`markdownLinks.ts`, shared with `MarkdownView`) and a link's title by the
viewer's rule (`refLinkTitle`), reduced to the elements a frame may draw
— `INLINE_ELEMENTS` for a line, plus the blocks for a body. A link is a
`ref` (the library draws a link that `navigate`s, Cmd/Ctrl-click a new
tab, its click stopped there), a web page's `href` (drawn as its text with
the URL on hover: a frame opens nothing off oxplow, which would be a way
out for its data) or `broken` (why, on hover); raw HTML, images, tables
and a `javascript:` URL never cross (`urlTransform`). No HTML crosses at
all: the library builds each node with `createElement`, and any tag
outside its own list leaves its text. The element gets the kit's `ox-md`
(and `ox-md-inline`, whose children flow in the element whatever its own
display). The app's host looks titles up in its caches (`liveTreeDeps`);
the test harness's host runs the same tree with none (`NO_LOOKUPS`) —
`markdownTree`, `markdownLinks` and `resourceStore` import no React, so
the harness stays React-free (`harnessBuild.test.ts`). The Work board's
cards draw their description this way. `oxplow extension new component <name>` scaffolds an
extension — the component, a page that shows it and a bundle that uses
the library — that checks and tests clean (`just_works.rs`). The
reference is in `docs/guide/pages.md` "Custom components".

**`custom_components` is stable**, on the evidence rule: a shared
extension's component acts with it. The github example's **PR Lifetimes**
(`components/pr-lifetimes/`, shown by a page) draws each pull request as
a bar from opened to merged — a range the kit's charts do not draw. Its
`state` filter is a param of the page, drawn in the page's rail. A bar opens `oxplow:github/pr?id=<n>`
(`navigate`), and Sync runs `oxplow.collector.sync` (`invoke`) — a
command outside the extension's namespace, so the manifest declares
`commands:<pattern>` for it. The host's own re-run (`onUpdate`) is
asked again with a selected filter rather than drawn over it. A pull
request whose dates cannot be read is left out and counted in a note,
never blanking the rest. A rejected `connect()` is said in the status. A
component author can use each of these as a dataset. The loader limits
components to no kind of extension. The browser suite's
`specs/components/pr-lifetimes.spec.ts` runs it in Chromium **and
WebKit** (the engines of browser mode and the macOS window; the suite's
one WebKit project): the frame loads, the filters and navigation work,
`invoke` is held to the declared commands, and a self-navigation off this
machine sends no request in either engine — the blocked page's load then
ends the component and its table shows.

**The kit's stylesheet.** `/component-lib/oxplow-kit.css`
(`assets/oxplow-kit.css`, served beside the library as `text/css`, and
named in every bundle's CSP `style-src`) is a few `ox-` classes in
oxplow's look — text, card, badge, buttons, forms (`ox-form`, `ox-field`,
`ox-label`, `ox-input`/`ox-select`/`ox-textarea` with the accent focus
ring, `ox-error`), a nesting panel (`ox-nest`: a parent and its children,
as a `depth` list draws them), a menu (`ox-menu`, `ox-menu-item`, `ox-menu-separator`,
`ox-menu-heading`, `ox-danger`), drag and drop (`ox-drag`, `ox-dragging`,
`ox-drop-target`, the Work board's dashed accent drop look), table,
states, chart series. They read only the theme's tokens (`kitSheet.test.ts`:
every `var(--x)` is one the app's root defines, and no literal color). A
bundle links it and calls `component.applyTheme()`, which sets each of
`init`'s `tokens` on the frame's root through the CSSOM — no `<style>`
text crosses the bridge.

**Check lints**: what a bundle's CSP refuses without a word, or what ends
the component, is an error at check, at its file (`bundle_problems` /
`page_problems`, from `check_components`):

- an inline `<script>` that runs (no `type`, or a JavaScript one — a
  `type="application/json"` data block is fine) or an event handler on a
  standard element (a custom element's `on…` attribute is its own);
- a `type="module"` script;
- a load from outside the bundle (`src`, each `srcset` candidate, a
  `<link>`'s `href`) — a scheme other than `data:`, `//host`, an absolute
  path other than `/component-lib/` (matched before `?`/`#`), or `..`
  past the folder;
- a `<base>`, an `<iframe>`/`<frame>`, an `<object>`/`<embed>`, a
  refresh `<meta>` (a `<form>` is allowed: it submits nowhere);
- an `index.html` that never loads the client library.

Only each bundle's **`index.html`** is linted — the one page a frame
shows, since a nested frame is refused and navigating away ends the
component. It is read from the **version under check**: the candidate of
a review (`ReviewSide::read`), the folder on disk for a plain check. A
version without the built page (a revision's tree) has nothing to lint;
loading reports a bundle with no `index.html`. A bundled extension's
pages are linted from its embedded files, the same way. The scan is a
start-tag reader, comments skipped and raw-text elements (`script`,
`style`, `textarea`, `title`) read to their close, not a full HTML
parser.
