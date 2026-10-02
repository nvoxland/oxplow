The collector `hot` of the extension `work` was disabled on this machine after it kept failing:

> 3 failures in a row; the last: collector `hot`: division by zero

## What it's for

Lists the high-priority tasks.

It was made in [[effort:eff12]].

## Its declaration

In `oxplow/extensions/work/extension.yaml`:

```yaml
id: hot
runtime: starlark
entry: hot.star
```

## Recent failures (newest first)

- collector `hot`: division by zero
- collector `hot`: division by zero

## What `oxplow plugin check` reports

- oxplow/extensions/work/extension.yaml:4: a warning

Its intent examples, which it must still satisfy: `lists one`.

It targets oxplow `>=0.7`; this is oxplow 0.7.0.

## What to do

1. Find why it fails: read the failures and its source, and reproduce one (`oxplow plugin test work`).
2. Fix it, keeping its purpose and examples.
3. Check it: `oxplow plugin check work` and `oxplow plugin test work` must be clean.
4. Say what you changed on this item. A person enables it again (`plugin.enable`, Settings → Extensions); you can't.
