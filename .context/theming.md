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

| Variable                | Value     | Used for                                   |
|-------------------------|-----------|--------------------------------------------|
| `--surface-app`         | `#14161a` | App background, page bodies                |
| `--surface-card`        | `#1c1f24` | Cards, inner content surfaces              |
| `--surface-rail`        | `#181b20` | Left HUD rail                              |
| `--surface-tab-active`  | `#1f2228` | Currently-focused tab body                 |
| `--surface-tab-inactive`| `#0e1014` | Tab strip and unselected tabs              |
| `--surface-elevated`    | `#232730` | Popovers, slideovers, kebab menus          |
| `--surface-overlay`     | rgba dim  | Backdrops behind slideovers / overlays     |

### Borders

| Variable          | Value     | Used for                              |
|-------------------|-----------|---------------------------------------|
| `--border-subtle` | `#2a2e36` | List dividers, card edges             |
| `--border-strong` | `#3a3f49` | Focus / hover outlines, tab frames    |

### Text

| Variable           | Value     | Used for                       |
|--------------------|-----------|--------------------------------|
| `--text-primary`   | `#e6e8ec` | Default body text              |
| `--text-secondary` | `#9aa1ad` | Captions, metadata             |
| `--text-muted`     | `#6c727c` | Placeholders, disabled         |

### Accent (primary action)

| Variable             | Value     | Used for                       |
|----------------------|-----------|--------------------------------|
| `--accent`           | `#5b8cf5` | Primary buttons, focus rings   |
| `--accent-hover`     | `#7aa2f7` | Hover variant                  |
| `--accent-soft-bg`   | `#1f2a44` | Active-pill / soft-button bg   |
| `--accent-on-accent` | `#ffffff` | Foreground on accent surfaces  |

### Status (semantic — work item / agent state)

`--status-running` (blue), `--status-waiting` (amber),
`--status-ready` (slate), `--status-human-check` (violet),
`--status-done` (emerald), `--status-canceled` (gray).

### Severity (code quality)

`--severity-low` (slate) → `--severity-medium` (amber) →
`--severity-high` (orange) → `--severity-critical` (rose).

### Freshness (notes)

`--freshness-fresh` (emerald), `--freshness-stale` (amber),
`--freshness-very-stale` (rose).

### Diff

`--diff-add-bg` / `--diff-add-fg`, `--diff-remove-bg` / `--diff-remove-fg`.

### Blame overlay

Two hue tracks (local amber / git blue), four saturation steps for age,
plus `--blame-uncommitted` and `--blame-{local,git}-border`.

### Legacy aliases (transitional)

Components written before the semantic-token migration still reference
`--bg`, `--bg-1`, `--bg-2`, `--bg-3`, `--bg-tab-inactive`, `--bg-detail`,
`--fg`, `--muted`, `--border`, `--priority-{urgent,high,medium,low}`.
**New components must use semantic tokens directly**, not the legacy
aliases. Aliases will be removed during the visual-polish phase as
each component is migrated.

## Density

Phase 7 (the visual-polish pass) tuned the app's density to
Metabase-grade rather than dense-IDE. The relevant numbers:

- **Body font** is 14px (was 13px). Captions/metadata stay at 13px;
  IDs/timestamps that need column alignment use the `.oxplow-tabular`
  class (12px tabular-nums).
- **List rows** (work items, file tree, notes, code-quality findings,
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
