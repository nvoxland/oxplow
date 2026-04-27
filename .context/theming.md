# Theming

What this doc covers: the semantic CSS-variable system that drives both
**light** (default) and **dark** themes, where each variable is meant to be
used, how the runtime switch works, and the rule for adding new ones.
Variables live in `public/index.html` under the
`:root[data-theme="light"]` and `:root[data-theme="dark"]` blocks.

## How the switch works

- The renderer's bootstrap (`src/ui/main.tsx`) calls
  `initTheme()` from `src/ui/theme.ts` before rendering. That reads the
  user's preference from `localStorage["oxplow.theme"]` (`light` /
  `dark` / `system`; default `light`), resolves it to a concrete theme
  (system queries `prefers-color-scheme`), and writes
  `data-theme="light|dark"` onto `document.documentElement`.
- Components never branch on theme. They reference CSS variables; the
  variables resolve to the active theme's value automatically.
- The `ThemeToggle` component in `src/ui/components/ThemeToggle.tsx`
  exposes the preference as a compact (single icon, cycles light → dark
  → system) or segmented (three side-by-side options) control. The
  StreamRail mounts the compact variant in its top-right area.
- Subscribing: `subscribeThemePref(cb)` notifies on changes; the
  toggle itself uses this to stay in sync if multiple instances
  are mounted.

## Token groups

All values are hex / rgba; both themes define every token. **Components
must reference variables only — never inline hex.** A new value goes in
both themes or it doesn't go in.

### Surfaces (background tiers)

| Variable                | Light     | Dark      | Used for                                   |
|-------------------------|-----------|-----------|--------------------------------------------|
| `--surface-app`         | `#f5f7fa` | `#14161a` | App background, page bodies                |
| `--surface-card`        | `#ffffff` | `#1c1f24` | Cards, inner content surfaces              |
| `--surface-rail`        | `#f1f3f7` | `#181b20` | Left HUD rail                              |
| `--surface-tab-active`  | `#ffffff` | `#1f2228` | Currently-focused tab body                 |
| `--surface-tab-inactive`| `#d4d9e3` | `#0e1014` | Tab strip and unselected tabs              |
| `--surface-elevated`    | `#ffffff` | `#232730` | Popovers, slideovers, kebab menus          |
| `--surface-overlay`     | rgba dim  | rgba dim  | Backdrops behind slideovers / overlays     |

### Borders

| Variable          | Light     | Dark      | Used for                              |
|-------------------|-----------|-----------|---------------------------------------|
| `--border-subtle` | `#e5e8ee` | `#2a2e36` | List dividers, card edges             |
| `--border-strong` | `#c4cad4` | `#3a3f49` | Focus / hover outlines, tab frames    |

### Text

| Variable           | Light     | Dark      | Used for                       |
|--------------------|-----------|-----------|--------------------------------|
| `--text-primary`   | `#1a1d23` | `#e6e8ec` | Default body text              |
| `--text-secondary` | `#5a6371` | `#9aa1ad` | Captions, metadata             |
| `--text-muted`     | `#8a92a0` | `#6c727c` | Placeholders, disabled         |

### Accent (primary action)

| Variable             | Light     | Dark      | Used for                       |
|----------------------|-----------|-----------|--------------------------------|
| `--accent`           | `#2563eb` | `#5b8cf5` | Primary buttons, focus rings   |
| `--accent-hover`     | `#1d4ed8` | `#7aa2f7` | Hover variant                  |
| `--accent-soft-bg`   | `#eef4ff` | `#1f2a44` | Active-pill / soft-button bg   |
| `--accent-on-accent` | `#ffffff` | `#ffffff` | Foreground on accent surfaces  |

### Status (semantic — work item / agent state)

| Variable               | Light hue   | Dark hue    |
|------------------------|-------------|-------------|
| `--status-running`     | blue        | blue        |
| `--status-waiting`     | amber       | amber       |
| `--status-ready`       | slate       | slate       |
| `--status-human-check` | violet      | violet      |
| `--status-done`        | emerald     | emerald     |
| `--status-canceled`    | gray        | gray        |

### Severity (code quality)

`--severity-low` (slate) → `--severity-medium` (amber) →
`--severity-high` (orange) → `--severity-critical` (rose).

### Freshness (notes)

`--freshness-fresh` (emerald), `--freshness-stale` (amber),
`--freshness-very-stale` (rose).

### Diff

`--diff-add-bg` / `--diff-add-fg`, `--diff-remove-bg` / `--diff-remove-fg`.

### Blame overlay (kept from prior theming, light-tuned)

Two hue tracks (local amber / git blue), four saturation steps for age,
plus `--blame-uncommitted` and `--blame-{local,git}-border`.

### Legacy aliases (transitional)

Components written before the semantic-token migration still reference
`--bg`, `--bg-1`, `--bg-2`, `--bg-3`, `--bg-tab-inactive`, `--bg-detail`,
`--fg`, `--muted`, `--border`, `--priority-{urgent,high,medium,low}`.
Those aliases are defined per theme and map to the semantic tokens.
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

## Monaco theme follows the app

`src/ui/monaco-theme.ts` exports `getActiveMonacoTheme()` and
`subscribeMonacoTheme(fn)` so the embedded code editors track the
app theme. `EditorPane.tsx` and `DiffPane.tsx` both call
`monaco.editor.setTheme` from a subscriber on mount; flipping the
ThemeToggle re-tints both editors live. Monaco theme ids: `vs` for
light, `vs-dark` for dark. The helper has no monaco import so it's
unit-testable from `monaco-theme.test.ts`.

## Xterm theme follows the app

`src/ui/xterm-theme.ts` mirrors the Monaco bridge for the embedded
terminal. Exports `getActiveXtermTheme()` and `subscribeXtermTheme(fn)`,
each returning an `XtermTheme` (xterm.js `ITheme` shape). Background,
foreground, cursor, and selection read from `--surface-card`,
`--text-primary`, `--accent`, and `--accent-soft-bg` at call time;
the 16 ANSI slots use vetted One Light / One Dark palettes hard-coded
in the helper (UI accents would clash with terminal-color conventions
like red=error/green=success). `TerminalPane.tsx` passes the active
theme into `new Terminal({ theme })` and assigns
`term.options.theme = next` from a subscriber so flipping the toggle
re-tints the live terminal — no re-mount.

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
value. The variable goes in both themes. Naming convention:

- `--surface-<role>` — background tiers and surface-specific backgrounds.
- `--text-<role>` — text colors.
- `--border-<weight>` — divider colors.
- `--status-<state>` / `--severity-<level>` / `--freshness-<state>` —
  semantic categories.

## Related

- `src/ui/theme.ts` — runtime preference + `data-theme` apply.
- `src/ui/components/ThemeToggle.tsx` — UI control.
- `public/index.html` — variable definitions.
