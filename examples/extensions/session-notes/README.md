# session-notes

Adds a section to a foundation page. The New Session page has a slot,
`oxplow-foundation/new-session.sections`, among its sections with stable ids
`start` and `earlier`; this
extension mounts its `thread-notes` page there `at: { after: start }`, so
the thread's notes show between starting a session and picking an earlier
one. The page binds `thread_id`, which the page declares.

Copy it to `.oxplow/extensions/session-notes` and open New Session in any
thread.
