# Theming

What this doc covers: the background-color tier system, where each
variable is meant to be used, and the rule for adding new ones. All
variables live in `public/index.html` under `:root`.

## Variables

| Variable             | Hex        | Used for                                           |
|----------------------|------------|----------------------------------------------------|
| `--bg`               | `#0e0e0e`  | Editor / main content (darkest, neutral)           |
| `--bg-1`             | `#1a1815`  | Center tab strip, side panels (faint warm)        |
| `--bg-2`             | `#342921`  | Batch rail (warm amber/sienna)                    |
| `--bg-3`             | `#2a313f`  | Stream rail (cool blue-tinted top chrome)         |
| `--bg-tab-inactive`  | `#3a3a3d`  | Non-selected batch tabs (neutral light gray)      |
| `--bg-detail`        | `#242424`  | Expanded-detail surfaces (neutral, no warm tint)  |
| `--fg`               | `#e6e6e6`  | Default text                                       |
| `--muted`            | `#8d8a85`  | Secondary text (warm-tinted to match chrome)      |
| `--border`           | `#2f2b26`  | 1px dividers                                       |
| `--border-strong`    | `#3c3832`  | Tab frames, panel borders                          |
| `--accent`           | `#4a9eff`  | Selection, focus, primary buttons, tab underline  |
| `--priority-urgent`  | `#e06c75`  | Work-item priority icon — urgent                   |
| `--priority-high`    | `#e5a06a`  | Work-item priority icon — high                     |
| `--priority-medium`  | `#8d8a85`  | Work-item priority icon — medium (alias of muted)  |
| `--priority-low`     | `#6b6864`  | Work-item priority icon — low (recedes past muted) |

## Tier rule

Lighter chrome = higher in the visual hierarchy (top of window). Editor
is darkest because that's where the eye spends most of its time and
contrast against text matters most.

The warm/cool split is intentional:

- Stream rail (top chrome) is **cool** — it picks up a tint that echoes
  the blue accent.
- Batch rail (under the stream rail) is **warm** — sienna/amber, the
  complementary side of the wheel. This makes the two rails clearly
  distinguishable even when adjacent.
- Center tab strip and side panels stay near-neutral so they don't fight
  either rail.

## Where to use which

- Anything that's "the content the user is working on" → `--bg`.
- Tab strips, side-panel chrome, modal headers → `--bg-1`.
- Batch rail and elements that should feel "inside" the batch chrome →
  `--bg-2`.
- Stream rail and elements that should feel "inside" the stream chrome →
  `--bg-3`.
- Inactive tabs (where you want the tab to read as a tab against the rail
  it sits on) → `--bg-tab-inactive`.
- Expanded detail panels nested inside warm chrome (e.g. the work-item
  detail inside the Plan pane) → `--bg-detail`. Neutral grey so the
  detail reads as "content surface" rather than picking up the warm batch
  tint from `--bg-2`.

Buttons that need to stand off from a colored rail use `--bg` (e.g. the
"+ New batch" button on the warm batch rail) so they read as proper
controls rather than blending into the chrome.

## When to add a new variable

If two surfaces share a `--bg-N` and you need to distinguish them
visually, **add a new variable** rather than inlining a hex value. The
recent example: when batch tabs and the batch rail were both `--bg-2`,
the inactive tab needed its own value, so `--bg-tab-inactive` was added.

Naming convention: `--bg-<tier>` for the elevation tiers, `--bg-<role>`
for purpose-specific surfaces (`--bg-tab-inactive`).

## Related

- `src/ui/components/CenterTabs/CenterTabs.tsx` — uses `--bg-1` for the
  strip, `--bg` for the active tab.
- `src/ui/components/BatchRail.tsx` — uses `--bg-2` for the rail,
  `--bg-tab-inactive` for non-selected tabs, `--bg` for the selected tab
  and for the small buttons (`+ New batch`, etc.).
- `src/ui/components/StreamRail.tsx` — uses `--bg-3` for the rail.
