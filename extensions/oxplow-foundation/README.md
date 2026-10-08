# oxplow-foundation

oxplow's own commands, declared the way any extension declares one. Each
entry under `commands:` names an operation of a scope
(`scope: bookmarks.write`, `op: set`) and says how the command meets
the people and agents who run it: its summary, who may run it, whether a
person confirms it, and how search offers it.

The operations themselves are oxplow's native code. An extension can
declare a command over the same operation under its own namespace; what
it may use is what its `needs` lists.

It is required: oxplow's other parts run these commands, so it can't be
disabled.
