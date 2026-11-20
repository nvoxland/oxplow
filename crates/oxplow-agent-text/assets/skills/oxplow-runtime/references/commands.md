# The command bus: families and config

- Settings commands: `oxplow.config.list_keys`,
  `oxplow.config.get { key, level? }`,
  `oxplow.config.set { key, value, level }`,
  `oxplow.config.unset { key, level }`. Set and unset require an explicit
  `local`, `project` or `global` level. Project overrides are flat namespaced
  keys in `.oxplow/project.yaml` (for example, `oxplow.agents`). Keys that run
  a program or pick the model need the person's confirmation. Extension scripts
  declare `needs: [config.read]` and read with
  `scope("config.read", {"key": "oxplow.agents"})`.
- Tasks `task.*` (when a list is active); test
  evidence `oxplow.test.record_run`, `oxplow.collector.sync`, ….
