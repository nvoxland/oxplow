---
description: Review the user's open follow-up comments and respond to them.
---

The user leaves comments anchored to text across pages (wiki bodies,
code file lines, task details). Some are just notes-to-self; the ones
marked **follow-up** want you to act.

Scope from `$ARGUMENTS`:
- empty or `thread` → review the **current thread**'s comments.
- `stream` → review every open follow-up across the whole stream
  (every page in this workspace). Never go cross-stream.

Steps:

1. Call `mcp__oxplow__list_comments` with the chosen `scope`
   (`id` = this thread's `b-…` id, or the stream's `s-…` id) and
   `status: "needs_response"`. Each result carries the anchored
   `quote`, the message thread, and the comment's `intent`.
2. If the list is empty, report "no follow-ups need a response" and
   stop.
3. Otherwise consider them **holistically** first — the user often
   collects related thoughts across several spots before asking you to
   look. Note any themes, then address each: read the anchored
   `quote` in its `target` (open the file/page), do the work the
   comment asks for, and reply with
   `run_command knowledge.reply_comment { comment, body }` summarizing
   what you did or answering the question.
4. When a comment is fully addressed, run
   `knowledge.update_comment { comment, status: "resolved" }`. Leave it open if
   you've replied but the user still needs to weigh in.

File a work item for any follow-up that turns into real shippable
work (per the `oxplow-runtime` skill) rather than doing large changes
straight from a comment reply.
