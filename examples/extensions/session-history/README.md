# session-history

Adds a section to a foundation page. The New Session page has a slot,
`oxplow/new-session.sections`, among its sections with stable ids
`start` and `earlier`. This extension mounts its `thread-history` page there
`at: { after: start }`, so the thread's earlier prompts show between starting
a session and picking an earlier one. The page binds the host's `thread_id`
and reads recorded prompts from `v_agent_turn`; turns with no prompt are omitted.

Copy it to `.oxplow/extensions/session-history`, deploy it, and open New Session
in any thread.
