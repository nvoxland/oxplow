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

Each message you send to an agent carries **one** basket. By default that is
**This Thread**. To send a named basket instead, open it in the **Context** panel
(the @ icon in the bottom bar) and press **Send Next** (the paper-plane button).
That basket's tab gets the paper plane, and the thread's next prompt carries
its items instead of This Thread's. After that the basket is empty, and the
thread goes back to sending This Thread. The basket itself stays until you
delete it. This Thread's items wait for a later prompt.

Each item remembers the stream you collected it in. When it goes to a thread in
another stream, the agent is told where it came from, so it reads the file from
the right worktree.

## Managing baskets

The **Context** panel has a tab for each basket: **This Thread** and each named
basket, with how many items it holds. The basket the thread's next prompt will
send has a small paper plane after its count.

- Each item is one row: its name, a note mark (white when it has a note, grey when not), the
  start of its text, and a × at the right to take it out.
- Click a row to open it in place: its whole text, its note, and **Open** and
  **Move to…**. Click the note to add or change it. Click the row
  again to close it.
- Right-click an item for **Move to**, to put it in another basket.
- Each tab's bar starts with **Send Next**, the paper-plane button. Hover over it
  to see what it does. On a named basket that is sending next, press it again
  to send This Thread instead.
- A named basket's bar also has **Rename**, **Clear** and **Delete**.
  Right-clicking its tab gives the same choices.
- **Clear** on This Thread's tab takes out all of its items.

After you add something, the panel opens the tab of the basket it went to. The
**Context** page (open it from the panel's header) shows the same baskets as
tables.

## Sent by mistake?

If a message carried a basket you did not mean to send, press **Put Back** on
the right of the Context panel's bar ("Sent 2 items from basket 1 · Put Back").
The items return to the basket they came from, with their notes. Put Back
doesn't choose that basket to send next, so your next message won't carry it
unless you press Send Next. Items that go back to This Thread are held: each
says "held, will not send", and they wait until you press Send Next on This
Thread. Adding something new doesn't send them. You can also just tell the agent "put that back". Only the last send can be put back: the next
message replaces it.

## What the agent gets

The next prompt to any agent in the thread carries one basket, with each
note: This Thread, or the basket you chose with Send Next. Items from a named
basket are listed under its name. Then those items are gone: the agent's
transcript is the record of what it got.

See [Agents](agents.md) for the rest of what oxplow tells an agent.
