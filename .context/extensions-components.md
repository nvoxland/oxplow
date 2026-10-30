# Extensions: Custom components and Explore Data

Custom components (the sandboxed frame, protocol, testing a component) and the Explore Data app. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## What works today: Explore Data

- **Explore Data: oxplow-foundation's data app.** Deliberately simple,
  and the proof that an extension ships a data app: a **page with a
  component node** (`pages/explore-data.yaml`: `view: { component: { id:
  explore-data } }`, the component declared under `custom_components:`,
  its source in `extensions/oxplow-foundation/src/explore-data/`, built
  with the Timeline by foundation's `package.json`). Core draws what the
  kit draws; the component holds only the editor and the pickers.
  - **The catalog** is the page's **rail**: a `rail:` node in the same
    page file (a `list` over `v_dataset`, oxplow's own first, its `view`
    column selecting the `dataset` param, the picked row emphasized). A
    pick pushes `#dataset=` (Back un-picks) and the component hears it.
  - **The component** (`main.ts`, its pure half `dataset.ts`, tested from
    the desktop as `viz/exploreDataset.test.ts`): the picked dataset's
    documented columns (`v_field`) and lineage (`v_dataset_lineage`:
    the datasets it reads, links that pick them, the tables it reads, the
    datasets that read it); the SQL box (a pick writes `SELECT * … LIMIT
    50`; any read-only query, metrics included — `SELECT bucket,
    MEASURE('<key>') FROM metric_grid('week')` — Cmd/Ctrl+Enter runs
    it); **Show as** (`table`, `list`, `number`, `markdown`, `bar`,
    `line`, `treemap`), a chart's columns picked from the result
    (`chartDefaults`, then a select per role); **Chart a metric** seeds
    the explorer's metric query (`metricTemplate`: `metricSeriesSql` as a
    line) and, while the SQL is still it, **Slice By** regenerates it with
    a dimension (`v_dimension`, `sliceTemplate`); edited SQL is never
    rewritten. Its view is the tab's `#` (`dataset`, `sql`, `viz`, `chart`,
    `template`), the unrun draft its history state; it ignores the echoes
    of its own writes, so only a real move (Back) redraws. "Ask your agent
    to build one…" invokes `oxplow.page.draft_with_agent`, the one command
    it declares.
  - **The result** is core's: the component hands the page an inline
    page (`showView`, protocol 12: its query as one `sql` loader, its viz
    over it — `specOf` in `src/explore-data/dataset.ts`), the page opens it
    (`open_page { specYaml }`) and draws it under the frame with
    `PageTree` (`pages/PageComponent.tsx`), live, with **Save
    as Page** (`pages/SaveViewForm.tsx`: `oxplow.extension.write_page {
    extension, page, spec, deploy: true }`, then opens the page; a saved
    page pins to a dashboard as an embed). A refused spec answers the
    component `INVALID` with its field, and it shows the reason under the
    SQL box.
  - **No raw tables.** The React page's Raw tables switch read physical
    tables over IPC; a component reads datasets only, as every page does.
    Raw reads stay the agent's `oxplow_query_sql`.
  - **What an agent shows.** An agent answers with a page rather than
    pasting rows: MCP **`oxplow_show_page { page | spec, params? }`** runs
    `oxplow.page.show`. A `spec` is an inline page of the agent's own
    (read-only, no actions or forms). It opens beside the conversation
    as a page file on `oxplow:temp` (`oxplow:oxplow/page?page=temp/<id>`) and the call returns the page's
    text. The person saves it with **Save to wiki** or **Save as Page**
    (`oxplow.extension.write_page`). Nothing else is stored: there are no
    thread answers and no Keep This. See pages-and-tabs.md "Showing a
    page".
  - The launcher's **Data** category holds Explore Data, Metrics
    and Dashboards.

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
name) bounds that to this machine. That is Tauri's CSP in the app, and a
one-directive meta CSP in `index.html` for a plain browser, where
whatever serves `dist/` sends no header (`viz/frameBound.test.ts` keeps
them one value; only `frame-src`, since a fuller policy would bound what
the page connects to, and a daemon over a tunnel is not on loopback).
The host also ends the component on its second `load`. So a component
reaches only what its page feeds it, its own read-only SQL (`sql:
read`) and the commands it may invoke (`commands`).

**A component that acts runs under its extension's permissions.**
`invoke` runs a command with the viewer's rights, so the commands a
component declares are the only ones it can run. A command outside its
extension's namespace needs `commands:<pattern>` in the manifest
(extensions-permissions.md "Permissions"; the loader refuses the extension otherwise). A person
approved those permissions when the extension was deployed. An `invoke`
of a command that the loaded bundle does not declare is `Invalid`. An
`invoke` from a version that is not the current one is `Denied` (the
frame must reload), and the host shows a refusal's
reason under the frame (`custom-component-refused`, `componentRefusal`)
— the component's own code may not. A component that declares no
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
enabled extension's bundles into the generation it publishes
(`PreparedComponentBundles`), each stamped with that generation. A frame
whose snapshot belongs to a generation that is no longer active has its
`querySql` and `invoke` refused. The host loads the bundle again on every
`extensionsChanged` (sent after an activation publishes,
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
    description: Open work items against the days left in a sprint.   # optional: one sentence (v_custom_component.description; none is made up)
    bundle: components/burndown        # default components/<id>; holds index.html
    commands: [oxplow.work_item.transition]
    sql: read                          # optional: its own read-only SQL (querySql); needs sql.read in permissions
```
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
isn't one of this extension's `custom_components`"; `<extension>/<id>` for
another extension's component is that extension's to have). A `viz:
custom` node needs `custom.component` and a `from` loader (its rows are
what the component shows and what an agent reads). An agent reads a
`viz: custom` node as its table, prefixed ``(custom component
`<ext>/<id>`; its table rendering)``, and a `component` node as
``(custom component `<id>`: it shows the page's loaders)``.
`check_extension` checks that the declared commands are registered, and
warns when a custom viz also fills a kit role block (`chart`, `tree`,
`timeline`, `steps`, `sections`, `hunks`, `code`, `diff`, `editor`, `embed`,
`terminal`) — the kit may already render it. That is the honest extent of
a "this reimplements the kit" lint.

**The bridged calls** (`viz_actions.rs`; the frame reaches them only
through the host). Each names the component by `id`, `<extension>/<component
id>` — the component itself, whatever shows it — and the bundle `version`
its frame was loaded at (`load_component { id }`). A version that is not
the current one is `Denied`. `invoke_component_command { id, command,
input, confirmed, version }` runs `command` when it is one of the
component's `commands` at that version, as **`Actor::Component {
component: id, on_behalf_of: Human }`** — the component acting for the
person, so every policy applies as if they ran it. It is audited as kind
`component` with the component's contribution ref as source
(`command.executed@5`). The input is literal (no placeholders). A command
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
`remoteBaseUrl()` null — the page shows its table), under a **custom**
badge. On the frame's first `load` the host posts `init { protocol, run,
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
command, input }`; `{ id, method: navigate, uri }` (an oxplow page only:
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
`height: auto`). `setTitle`, `setActions` and `showView` are a
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
  stale or pulls in React) mounts the frame with the app's own
  `createBridgeHost` / `initMessage` / `pageData`. What the core answers —
  `invoke` (`invoke_component_command`, as the person), `querySql`
  (`run_component_sql`), `openForm` / `askForm`, `showView` — runs on the
  test's oxplow; what only the shell answers (`setTitle`, `setActions`,
  `setViewParams`, `setHistoryState`, `navigate`, a confirmation) is
  recorded, in the order made (one queue of posts, which the next op waits
  for). `showView` runs one at a time in the order asked and is recorded
  once the core has answered: `shown` is the spec when the core drew it,
  `null` when it refused it or the component cleared it. The server drives
  it one op at a time (`/harness/next`): a DOM step for the agent, a nav-bar
  press, a view change, new rows.
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
  input, ok, result | refused }], navigated, confirmed, forms, shown }`,
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
narrow window) is capped and scrolls, so the body keeps the height.

**Whose view a component has** (protocol 9, `viz/hostedView.ts`
`useHostedView`): its host's, where the host shares one, and only what
the host's contract names.

- A page that **is** the component (`component` as the whole view) has
  the whole entry: all its params (`patchViewParams`) and its history
  state.
- A **replacement** has the host page's view params that its target's
  contract lists (`UiReplacement.view`, `v_replaceable.view`: Git
  History's `q`, `author`, `branch` for the history graph). These are the
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
(null forgets), scope `component:<ext>/<id>` in `ui_preference` — for
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
that asks shows the host's `CommandConfirm`; confirming re-runs it
confirmed, declining answers `CANCELLED`. No `ready` within 3 s, or a
second `load` (the frame navigated itself), tears it down and shows the
table. `ready` records `usage { kind: "custom_component", key: <ext>/<id>
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
unsubscribe, invoke(command, input), navigate(uri, { newTab, replace }),
querySql, openForm (a command's, or `{ title, schema, values }`),
signIn, cancelSignIn, setTitle, setActions, showView, onAction,
viewParams, setViewParams, onViewParams, historyState, setHistoryState,
onHistoryState, pageMemory, applyTheme(doc?) }` (the `.d.ts` beside it
is the reference; `viewParams` / `historyState` are null where the host
gives no view). A failed request rejects with the host's `{ code,
message }`. `init` carries `protocol` (`BRIDGE_PROTOCOL` in
`componentBridge.ts`, `PROTOCOL` in the library — one number, bumped when
a message's shape changes; now 12), and `connect` rejects a host that
speaks another. One app is one library version. `componentClient.test.ts`
drives the served file against the real `createBridgeHost` and the real
`init` (`initMessage`, which `CustomComponentViz` posts), so the two
cannot drift. `oxplow extension new component <name>` scaffolds an
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
ring, `ox-error`), drag and drop (`ox-drag`, `ox-dragging`,
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
