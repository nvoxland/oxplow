You're in an empty project that uses oxplow. Build an oxplow extension
named `stale-work` that answers: which open tasks has nobody touched for a
week?

Collect them with a Starlark collector over the task data, publish a model
over what it collects, and show the model in a lens.

Start with `oxplow plugin new`. Run `oxplow plugin check stale-work` and
`oxplow plugin test stale-work` until both are clean — no errors and no
warnings. Don't sync, approve or enable anything; you can't.
