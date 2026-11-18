# Collecting context for an agent

A context basket holds what you collect for an agent: pages, selected text and
rows. Each thread has its own basket, **This Thread**. You can also keep named
baskets, one per topic, and send one to a thread when you are ready.

## Adding things

- **A page.** Open its ☰ menu and choose **Add to Context**.
- **Selected text on any page, in an editor, a diff or a terminal.**
  Right-click the selection and choose **Add Selection to Context**. In an
  editor with nothing selected, **Add File to Context** adds the file. In a
  terminal where a program such as Claude Code has the mouse, hold Option (⌥)
  while you drag to select.
- **A row.** Right-click it and choose **Add to Context**.

## The note and the basket

Every add opens the **Context** panel (the @ icon in the bottom bar) with a
card for the new item:

- **Note** (optional). Write what you want done about the item. The agent gets
  the note with the item and acts on it. Because it goes with the item, it
  stays right even when the code moves.
- **To.** The basket it goes in: **This Thread**, one of your named baskets, or
  **+ New basket**, which asks for a name. To starts at the basket you used
  last, so you can add several things to one basket in a row.

Press Ctrl/Cmd+Enter or **Add** to add it. Escape or **Cancel** adds nothing.

This Thread needs a selected thread. A named basket does not: you can collect
first and pick a thread later.

## Named baskets

Named baskets belong to the project, not to one stream or thread. Collect into
one while you read in one stream, then send it to a thread in another.

To send a named basket, **attach** it to a thread. Open the **Context** panel
(the @ icon in the bottom bar) and choose **Attach** on the basket. The
thread's next prompt carries This Thread's items and the attached basket's.
After that the basket is empty and no longer attached. The basket itself stays
until you delete it.

Each item remembers the stream you collected it in. When it goes to a thread in
another stream, the agent is told where it came from, so it reads the file from
the right worktree.

## Managing baskets

The **Context** panel has a tab for each basket: **This Thread** and each named
basket, with how many items it holds. A basket attached to this thread has a
link mark on its tab, and This Thread's tab names it too.

- Each item shows its name (click it to open where it came from), the text you
  selected, and your note. Hover over an item to add a note, or click the note
  to change it. Take an item out with its ×.
- Right-click an item for **Move to**, to put it in another basket.
- A named basket's tab has **Attach to This Thread** or **Detach**, **Rename**,
  **Clear** and **Delete**. Right-clicking its tab gives the same choices.
- **Clear** on This Thread's tab takes out all of its items.

After you add something, the panel opens the tab of the basket it went to. The
**Context** page (open it from the panel's header) shows the same baskets as
tables.

## What the agent gets

The next prompt to any agent in the thread carries This Thread's items and
every basket attached to the thread, each with its note. Items from a named
basket are listed under its name. Then those items are gone: the agent's
transcript is the record of what it got.

See [Agents](agents.md) for the rest of what oxplow tells an agent.
