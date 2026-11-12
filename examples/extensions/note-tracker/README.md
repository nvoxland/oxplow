# note-tracker

A task tracker written as a script. `trackers/notes.star` defines one
Starlark function per task tracker verb (`create`, `update`,
`transition`), and `extension.yaml` declares it as an implementation of
the `task_tracker` capability:

```yaml
implementations:
  - capability: task_tracker
    id: notes
    entry: trackers/notes.star   # a script, not a built-in
    needs: [sql.read]            # the scopes it calls
    stages: [...]                # its workflow, as a provider declares it
```

Choose it in Settings → Capabilities (or `oxplow.capability.choose {
capability: task_tracker, provider: notes }`). The task commands then call
its functions through the same dispatch they use for oxplow's own tracker
and for a provider's program.

Each function gets one dict — `input` (the verb's input), `config` (the
entry's `config:`), `id` (`notes`) and `key` (the call's idempotency key)
— and answers `{ result, events?, inverse? }`. The tracker keeps nothing
of its own: each answer records the note's whole row as a
`task.recorded@3` event, and a later verb reads the row back from
`v_task`.

`oxplow extension test` runs its fixture and the task tracker conformance
suite against it.
