# Page layout: where things go

oxplow draws every page the same way, so a page you write looks and
works like oxplow's own. You say *what* the page has; oxplow decides
*where* it goes. `check`, the extension's warnings and the deploy result
tell you when a page breaks these rules, each with the fix.

## The four places

| Place | What goes there | You write |
|---|---|---|
| **Nav bar** (the title row) | Actions on the whole page: Refresh, Copy, Add to Agent Context, Improve with Agent, and your lens's `actions` with no `row` or `group`. | `actions:` |
| **Body** | The title, the description, then content: sections, tables, charts, forms. The only controls are on rows: a row's `select`, a row's actions (its menu), links. | `viz`, `sections`, `columns`, `empty` |
| **Rail** (the side panel) | Filters: every param that narrows what the page shows. Selection: the row a click picked. About this page: your extension and its source file. | `params:` |
| **Nowhere** | A value a click on the page sets. It is page state, never an input. | `columns[].select` |

There is no toolbar and no `place:` key. A param you declare is a
filter in the rail, unless a column `select`s it: then it is the page's
selection, and the rail shows the picked row instead of an input.

## Filters

- **Each filter is a row**: its label beside its value. The person
  clicks the value to change it. Give every param a short `label`
  ("When", "Search subjects", "Merges only"); without one, its name is
  shown in words (`author_name` → "Author name").
- **Order is the order you declare.** Put the filter people change most
  first (usually the time range or the search), choices next, toggles
  last. The rail never reorders itself.
- **Six at most.** Past six, the rest fold behind "More". Fold related
  toggles into one `choice` (`show: open | closed | all`, not two
  `bool`s), or split the page.
- **Every filter has a sensible default**, so the page shows something
  useful the first time it opens. A `choice` needs one.
- **Pick the type that fits; never a wide control in a narrow place.**
  Free text is a `search`. A time span is a `date_range` (its custom
  dates open over the page). A list with more than about a dozen values
  is a `search`, not a `choice`.
- **Read every param in a query.** A filter that changes nothing is a
  bug the person sees.

## Selection and detail

- To show the detail of a row, put the list and the detail in one
  `document`: the list's key column has `select: <param>`, the detail
  section's query reads `:<param>`, and the document declares the param.
- The detail section's `empty:` says what to click ("Click a commit to
  see its files.").
- Don't show a selection as a param anyone types into.

## Body content

- **Description**: one or two sentences on what the page shows. Longer
  text goes in a `markdown` section.
- **Sections**: eight at most in a document; split a longer page.
- **Columns**: eight at most in a table or list. Give each one a
  `label`. Put long or extra fields in a detail section a row selects.
  A column with a `width` is cut to one line, with the whole value on
  hover, so give `width` to short, fixed-size values (ids, dates,
  counts).

## What check warns about

| Warning | Fix |
|---|---|
| more than 6 filters | fold related ones into one `choice`, or split the page |
| a `choice` with more than 12 options | use a `search`, or a lens that lists them |
| a param no query of the page reads (a column's `select` or `highlight` counts) | remove it, or read it as `:name` |
| a description over 200 characters | keep a sentence or two; move the rest to a markdown section |
| a document with more than 8 sections | split the page |
| a table or list with more than 8 columns | show the rest in a section a row `select`s |

A warning doesn't stop the page from loading, but fix it before you tell
the person the page is ready.

## State: a page's state is its params

A page's view — its filters, its search, the row picked — is its params,
and oxplow keeps them in the page's tab. A bookmark, a reopened tab, a
restart and Back all show the page the same way, and an agent reading
the open page sees the same values. So:

- **Declare what the page shows by as params**, with defaults. Only the
  values that differ from a default go into the page's ref.
- **A filter replaces; a pick pushes.** You don't choose: a param a
  column `select`s is the selection (Back un-selects it); every other
  param is a filter (no history entry per change).
- **Never keep view state anywhere else.** A custom component sets its
  page's view params with `component.setViewParams({ k: v })` and draws
  from `component.onViewParams(cb)`; this step's own memory with
  `setHistoryState` / `onHistoryState`; its own memory for the person,
  in every tab, with `component.pageMemory.get/set`. It never uses `localStorage` or
  `sessionStorage` (`check` warns). What only that page's entry needs
  to remember (scroll, open sections) oxplow keeps for the kit's
  components already.
- **A component writes its view on every change and follows it back.**
  Call `setViewParams` when what it shows changes (a drag, a zoom, a
  button), and redraw in `onViewParams` when Back or Forward moves the
  tab; skip a value it already shows. A default it computes ("the last
  week") stays out of the params, so a bookmark doesn't freeze it to one
  date. oxplow-foundation's Timeline does exactly this with `from`/`to`.
- **A component inside a page has its host's view, or none.** As a
  replacement it gets the keys its target lists (`SELECT target, view
  FROM v_replaceable`; the history graph: `q`, `author`, `branch`); on a
  lens's own page, the lens's params. In a dashboard tile or a slot
  `component.viewParams` is null: keep the filters in the component.
  A hosted component's history state is its own, apart from the page's.
- **The page never opens itself.** Another mode or "View all" is a param
  on the same page, not a second page.

## Where a page comes from

`oxplow_get_open_page` and `oxplow_read lens:<id>` give `origin.source`:
the file to edit, relative to the project. Edit that file, never
`origin.running`: that is the deployed copy, and the next deploy
replaces it. `outOfDate: true` means the source changed after the deploy
that runs; deploy it so the person sees the change.
