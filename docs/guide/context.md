# Collecting context for an agent

The context basket holds what you collect for a thread's agents. Pages, selected
text and rows go in. Everything that waits goes with the thread's next prompt.

## Adding things

- **A page.** Open its ☰ menu and choose **Add to Context**.
- **Selected text on any page, in an editor, a diff or a terminal.**
  Right-click the selection and choose **Add Selection to Context**. In an
  editor with nothing selected, **Add File to Context** adds the file. In a
  terminal where a program such as Claude Code has the mouse, hold Option (⌥)
  while you drag to select.
- **A row.** Right-click it and choose **Add to Context**.

Add to Context needs a selected thread.

## The note

Every add opens a small prompt with an optional note. Write what you want done
about the item, or leave it empty. Press Ctrl/Cmd+Enter or **Add** to add it.
Escape or **Cancel** adds nothing.

The agent gets the note with the item and acts on it. Because it goes with the
item, it stays right even when the code moves.

## Changing a note

Open the **Context** panel (the @ icon in the bottom bar) and edit the note
under the item. Or open the **Context** page from the panel's header and choose
**Edit Note…** on a row. You can change a note only while the item waits.

Take an item out with its ×. **Clear** removes them all.

## What the agent gets

The next prompt to any agent in the thread carries the basket, with each note.
Then the basket is empty. The Context page keeps a record of what was
delivered.

See [Agents](agents.md) for the rest of what oxplow tells an agent.
