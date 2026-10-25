# Task views: your own Board, list and editor

Three views that stand in for oxplow's own, with no work-items provider of
their own. They read `v_work_item` and change items only through the
work-item commands, so they work with whichever list is active.

| Replaces | Page | What it is |
|---|---|---|
| `work_item.board` | `swimlanes` | A `viz: custom` component: a column per state, cards you drag |
| `work_item.list` | `compact-list` | A table with title (a page link), state and updated; row actions **Done** and **Edit…** |
| `work_item.detail` | `edit-item` | A `viz: form` over `oxplow.work_item.update`: title and body, `uri` fixed |

```yaml
ui:
  replacements:
    - { target: work_item.board, page: swimlanes, version: 1 }
    - { target: work_item.list, page: compact-list, version: 1 }
    - { target: work_item.detail, page: edit-item, version: 1 }
```

## What the swimlanes do

- One column per canonical state: todo, in progress, blocked, done,
  canceled. Drag a card to another column and it runs
  `oxplow.work_item.transition`.
- Drag a card within its column and it runs `oxplow.work_item.reorder`,
  but only when the active provider's `features` include `ordering`
  (the page's `features` loader). Without it the drop does nothing.
- Click a title to rename the item: an inline form (Enter saves, Escape
  puts it back) that runs `oxplow.work_item.update`. A refusal naming a
  field shows under the input.
- Click a card to open the item; Cmd/Ctrl-click opens it in a new tab.
- If the board's commands are refused (the extension isn't deployed or is
  disabled), changing an item fails; the board says so in one line above
  the cards.

## What a view may and may not do

May:
- read any published model (`v_work_item`, `v_thread`, …) through its
  pages' loaders;
- change work items through the commands it declares in
  `custom_components[].commands` (a table or form on a page uses `actions` /
  `form`, which run as you);
- hold forms, drag and drop and its own filters, styled with the kit
  (`ox-form`, `ox-input`, `ox-error`, `ox-drag`, `ox-drop-target`, …).

May not:
- reach the network, the files or another page's state: a component runs
  in a sandboxed frame and talks to oxplow only through the client
  library;
- run a command it didn't declare (`custom_components[].commands`) or one
  outside the extension's `permissions.commands`;
- replace the page chrome around it (the scope picker, the rail, the
  backlog drawer stay oxplow's), or the work list itself.

## Try it

1. Copy this folder to `.oxplow/extensions/task-views/`.
2. Settings → Integrations: choose **task-views** for the board, list and
   editor (it's the only candidate, so it is the default).
3. Deploy it (`oxplow.extension.deploy { name: "task-views" }`, or Deploy on
   Settings → Extensions) and approve the work-item commands it asks for,
   so cards can change items.

`oxplow extension check task-views` lints it and `oxplow extension test
task-views` runs `fixtures/`: the list over some rows, the board's loaders
and a refused update, and the editor's loader.
