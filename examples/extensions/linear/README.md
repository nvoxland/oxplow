# linear — Linear issues as oxplow work items

The reference external work-items provider: a native program
(`crates/oxplow-provider-linear`) speaking oxplow's provider protocol.
A Linear team's issues become work items (`work_item:linear:ENG-12`):
they sync into `v_work_item`, show on the Board and in the agent's
filing, and every `work_item.*` write — create, update, transition,
link, comment, delete — goes to Linear.

## Try it

1. `scripts/install-linear.sh <project>` builds the provider and copies
   this folder, with the binary, into the project.
2. Settings → Extensions: enable `linear`. Settings → Data → Programs:
   approve its program (it reaches only `api.linear.app`, with the one
   credential `LINEAR_API_KEY`).
3. Settings → Integrations: set `LINEAR_API_KEY` (a personal API key from
   Linear → Settings → API; it stays in your keychain) and the team's key,
   Check, then Enable. Issues arrive with the first sync; "Active for work
   items" makes new items Linear issues.

## Config

| key | |
|---|---|
| `team` | The team's key, as in its issue ids (`ENG`). Required. |
| `project` | Only this project's issues; new issues go in it. |
| `blocked_state` | The workflow state that means blocked (default `Blocked`); the team must have it. |

States map by type: triage, backlog and unstarted are todo; started is
in progress; completed is done; canceled is canceled; the blocked state
is blocked. `link_type` is `blocks`, `relates_to` or `duplicates`.
Delete moves the issue to Linear's trash.

## Tests

`cargo nextest run -p oxplow-provider-linear` runs the provider against
an in-process simulator of the operations it sends, and this folder
through `oxplow plugin test` (`tests/kit.rs`; `OXPLOW_BLESS=1` rewrites
`fixtures/transcripts/linear.jsonl`). Against your own workspace,
`oxplow plugin test linear --bless` with `LINEAR_API_KEY` set records
your transcript.
