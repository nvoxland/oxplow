# Theming


What this doc covers: the semantic CSS-variable system that drives
oxplow's dark theme, where each variable is meant to be used, and the
rule for adding new ones. Variables live in `public/index.html` under
the `:root` block. **Oxplow is dark-only** — there is no light theme
and no runtime toggle.

## How it works

- The variables are declared once on `:root` in `public/index.html` with
  `color-scheme: dark`.
- Components never inline hex; they reference CSS variables.
- Monaco editors hard-code `vs-dark`; the embedded xterm uses a fixed
  dark palette in `TerminalPane.tsx`.

## Token groups

All values are hex / rgba. **Components must reference variables
only — never inline hex.**

### Surfaces (background tiers)

Cool blue-grey, three tonal tiers. `--surface-tab-inactive` is
`transparent` so the tab strip blends into whatever surface it sits
on — it's the active tab that lifts forward, not the strip that
sinks.

| Variable                | Value         | Used for                                   |
|-------------------------|---------------|--------------------------------------------|
| `--surface-app`         | `#0f1115`     | App background, page bodies (darkest content tier) |
| `--surface-card`        | `#161a20`     | Cards, inner content surfaces, rail panels |
| `--surface-chrome`      | `#1b2129`     | Lighter **chrome frame** around the dark content — rail aside, Navigator strip, content-panel surround. The content recesses below it. |
| `--surface-rail`        | `#13161b`     | Legacy rail tier (PageNavBar fallback, TerminalTabStrip, TaskDetail) — chrome surfaces moved to `--surface-chrome` |
| `--surface-tab-active`  | `#1c2027`     | Currently-focused tab body                 |
| `--surface-tab-inactive`| `transparent` | Inactive tabs (let the header tint show through) |
| `--surface-elevated`    | `#20242c`     | Popovers, slideovers, context menus        |
| `--surface-overlay`     | rgba dim      | Backdrops behind slideovers / overlays     |

**Panel headers.** `--panel-header-bg` (`rgba(108,156,246,0.12)`, a muted
accent wash — *not* grey) tints the header band of every panel so titles
read as intentional headers in dark mode: rail section headers
(`RailSection`), Navigator stream-header rows, the page tab bar
(`CenterTabs`), and the common page chrome (`PageNavBar`). It's
translucent, so when a surface needs the same look over a non-card base
(tab bar, page nav) layer it over `--surface-card`:
`background: linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)`.

### Borders

| Variable          | Value     | Used for                              |
|-------------------|-----------|---------------------------------------|
| `--border-subtle` | `#232831` | List dividers, card edges             |
| `--border-strong` | `#333944` | Focus / hover outlines, tab frames    |

### Text

| Variable           | Value     | Used for                       |
|--------------------|-----------|--------------------------------|
| `--text-primary`   | `#e8eaef` | Default body text              |
| `--text-secondary` | `#9097a3` | Captions, metadata             |
| `--text-muted`     | `#5f6571` | Placeholders, disabled         |

### Accent (primary action)

| Variable             | Value     | Used for                       |
|----------------------|-----------|--------------------------------|
| `--accent`           | `#6b9cf6` | Primary buttons, focus rings   |
| `--accent-hover`     | `#8db2f8` | Hover variant                  |
| `--accent-soft-bg`   | `#1c2a48` | Active-pill / soft-button bg   |
| `--accent-on-accent` | `#ffffff` | Foreground on accent surfaces  |

### Buttons

`--button-primary-bg` / `-fg` / `-bg-hover` for the *one* primary CTA
per region (e.g. `+ New stream`, `+ New thread`, Save). Secondary
buttons use `--button-secondary-*` (transparent background, neutral
text, lifts to a 4%-white wash on hover).

### Status (semantic — task / agent state)

`--status-running` (blue), `--status-waiting` (amber),
`--status-ready` (slate), `--status-human-check` (violet),
`--status-done` (emerald), `--status-canceled` (gray).

### Severity (code quality)

`--severity-low` (slate) → `--severity-medium` (amber) →
`--severity-high` (orange) → `--severity-critical` (rose).

### Freshness (notes)

`--freshness-fresh` (emerald), `--freshness-stale` (amber),
`--freshness-very-stale` (rose).

### Metric status (data-driven — NO hardcoded ramps)

Metric red/green is computed **from the metric definition's data**, not from
magic numbers in the renderer. `MetricsPage.statusColor(def, value)` reads
`def.target` / `def.fail_at` / `def.direction` and returns three tiers — meets
target → `--ok` (green), past the fail floor → `--err` (red), in-between →
`--warn` (amber); `neutral` / threshold-less metrics are uncolored. The former
hardcoded **coverage 50/80 ramp is retired** (tsk220): the thresholds now live on
the `oxplow.coverage.diff_pct` definition (`target: 80`, `fail_at: 50`, set in
`collection.rs::record_coverage_metric`). To re-tune a metric's coloring, change
its `target`/`warn_at`/`fail_at` (config or producer) — never a UI constant. (The
legacy `EffortObservations` coverage bar still restates `80`/`50` as named
constants documented to mirror the def; it's removed with `effort_observation` in
tsk215.)

### Comments

`--comment-highlight` (amber) — the inline highlight + quote rule for
text a comment is anchored to (margin-note feel).
`--comment-highlight-bg` is the translucent fill painted under the
anchored range in the editor / prose surfaces.

### Diff

`--diff-add-bg` / `--diff-add-fg`, `--diff-remove-bg` / `--diff-remove-fg`.

### Blame overlay

Two hue tracks (local amber / git blue), four saturation steps for age,
plus `--blame-uncommitted` and `--blame-{local,git}-border`.

### Legacy aliases (transitional)

Components written before the semantic-token migration still reference
`--bg`, `--bg-1`, `--bg-2`, `--bg-3`, `--bg-tab-inactive`, `--bg-detail`,
`--fg`, `--muted`, `--border`, `--priority-{urgent,high,medium,low}`.
The aliases now resolve to the unified semantic tier (e.g.
`--bg-2 → var(--surface-tab-active)`), so unmigrated components pick
up the cool palette automatically — no more warm-brown rails. **New
components must use semantic tokens directly**, not the legacy
aliases. Aliases will be removed entirely once every reference has
migrated.

## Density

Phase 7 (the visual-polish pass) tuned the app's density to
Metabase-grade rather than dense-IDE. The relevant numbers:

- **Body font** is 14px (was 13px). Captions/metadata stay at 13px;
  IDs/timestamps that need column alignment use the `.oxplow-tabular`
  class (12px tabular-nums).
- **List rows** (tasks, file tree, notes, code-quality findings,
  snapshots, commits) use `padding: 8–10px 12px` and target ~36–40px
  height — up from the prior ~24–28px.
- **Section headers** use `padding: 10px 12px` and 11px uppercase
  labels, against `--surface-app` so they read as a divider band
  rather than a card surface.
- **Tab strips** (`CenterTabs`) use `min-height: 36px` with
  `padding: 10px 14px` per tab.
- **Page chrome** (`Page.tsx`) header is ~56px tall (`min-height: 56px`,
  `padding: 14px 20px`); page titles are 17px / `font-weight: 600`.
- **Selection / marked rows** use a 3px left stripe (was 2px) plus
  `--accent-soft-bg` rather than a generic semi-transparent yellow.

When adding a new list surface, match these numbers — the
"Metabase-clean" feel relies on them being consistent across panels.

## Typography

Three font faces, one shared size/leading/weight scale. All defined as
tokens on `:root` in `apps/desktop/index.html`; components must read
the tokens, never inline a font stack.

| Variable        | Used for                                                          |
|-----------------|-------------------------------------------------------------------|
| `--font-ui`     | UI chrome — buttons, menus, labels, tabs, panel headers, sidebars |
| `--font-prose`  | Long-form body content — `.oxplow-md` (wiki bodies, task descriptions, effort summaries, anywhere `MarkdownView` renders) |
| `--font-mono`   | Code-shaped surfaces — Monaco, xterm, code blocks, blame columns, file-path chips, tabular IDs, stacktraces |

The prose-vs-UI-vs-mono split mirrors Wikipedia / Metabase: sans for
chrome, serif for article-style content, mono only for code. UI chrome
size scale (`--text-base: 14px`) is unchanged from the phase-7 density
tune-up — adding tokens is purely additive.

The three faces are **locally bundled** as variable woff2s under
`apps/desktop/public/fonts/` (served from `/fonts/` at runtime) and
declared via `@font-face` rules at the top of `apps/desktop/index.html`:

- **Inter** (`InterVariable.woff2` + `-Italic.woff2`) — UI face.
- **Source Serif 4** (`SourceSerif4Variable-Roman.woff2` + `-Italic.woff2`) — prose face.
- **JetBrains Mono** (`JetBrainsMonoVariable.woff2` + `-Italic.woff2`) — mono face.

All six are variable fonts (single file per style covers the full
weight axis) and ship Latin / Cyrillic / Greek / Vietnamese glyph
coverage out of the box. Total bundle weight is ~1.7 MB. Each
`--font-*` token lists the local family first then falls through to a
system stack as a safety net while the woff2 is still loading or if a
file goes missing.

Size scale: `--text-xs` (12px) → `--text-sm` (13px) → `--text-base`
(14px, UI default) → `--text-md` (15px, prose default) → `--text-lg`
(17px) → `--text-xl` (20px) → `--text-2xl` (24px) → `--text-3xl`
(28px).

Line-heights: `--leading-tight` (1.25, headings), `--leading-snug`
(1.4, dense UI rows — body default), `--leading-prose` (1.6,
long-form body).

Weights: `--weight-regular` (400), `--weight-medium` (500),
`--weight-bold` (700).

`--mono` is kept as a legacy alias for `--font-mono` so any older
references continue to resolve; new code should use `--font-mono`.

### `.oxplow-md` — the prose surface

Every `MarkdownView` is rendered inside an `.oxplow-md` wrapper. The
class caps width to 78ch (centered), switches the body to
`--font-prose` at `--text-md` / `--leading-prose`, applies a
restrained sans-serif heading scale with an H2 underline, and
restyles `code` / `pre` / `blockquote` / `hr` against the surface
tokens. Code surfaces inside prose stay on `--font-mono` —
prose-vs-code contrast is the whole point. Rules live in
`apps/desktop/index.html` next to the GFM-table rules.

### xterm and Monaco

xterm's `fontFamily` is a one-shot constructor option, so
`TerminalPane.tsx` resolves `--font-mono` once at mount via
`getComputedStyle(document.body)` and passes the resolved string to
`new Terminal({ ... })`. If the token swaps, remount the terminal to
pick it up — there is no live binding. Monaco does not set
`fontFamily` explicitly; it uses its internal default mono stack,
which is metrically identical to `--font-mono`. If a future change
needs Monaco to follow a custom face, do the same `getComputedStyle`
trick in `EditorPane.tsx` before `monaco.editor.create`.

## Monaco and xterm

Both embedded editors are dark-only. `EditorPane.tsx` and
`DiffPane.tsx` pass `theme: "vs-dark"` to Monaco; `TerminalPane.tsx`
defines `XTERM_THEME` inline (One Dark ANSI palette + surface/text
hex). No runtime swap, no subscriber wiring.

## Color use rules

- **Backgrounds and chrome stay neutral.** No saturated color on rails,
  tabs, or page surfaces.
- **Semantic color appears only where it carries meaning** — status
  pills, severity badges, freshness chips, diff backgrounds, charts.
- **Hover states** lighten/darken by ~4% rather than introducing a new
  hue.
- **Don't pair more than two accent hues per page** (the page's primary
  status + one accent). Dashboards may show more because they're
  data-display surfaces.

## When to add a new variable

If two surfaces need to look different and no existing token captures
the distinction, **add a new variable** rather than inlining a hex
value. Naming convention:

- `--surface-<role>` — background tiers and surface-specific backgrounds.
- `--text-<role>` — text colors.
- `--border-<weight>` — divider colors.
- `--status-<state>` / `--severity-<level>` / `--freshness-<state>` —
  semantic categories.

## Related

- `public/index.html` — variable definitions.
