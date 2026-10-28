# Inline pages: `oxplow-page` blocks and `oxplow_show_page`

An **inline page** is a page you write whole, as one-off YAML, to answer a
question or to live in a wiki page. It has no file and no extension.

## The block

In markdown (a wiki page) write a fenced block:

````markdown
```oxplow-page
title: Failing tests
loaders:
  t: { sql: "SELECT name, status FROM v_test_case WHERE status = 'failed'" }
view: { viz: table, from: t }
```
````

The wiki renders the block as the page, live (a wiki page opened in the app,
or `oxplow_run_page`'s text). In any other store it reads as the YAML. The
same YAML works as the `spec` of `oxplow_show_page`.

## What it may not do

It **only reads**. An inline page has no `command` loaders (use `sql`,
`rows` or `page` loaders), no `actions` (page or row), and no `form`,
`approvals`, `terminal`, `editor`, `custom` viz, `component` or `slot`
nodes. `oxplow_show_page` and the parser refuse each with the node named.
Nothing in it can change anything; an agent's one-off cannot act as the
person.

## `oxplow_show_page`

- `oxplow_show_page { page, params }` opens an existing page beside the
  conversation; `{ spec: "<yaml>" }` opens an inline one. It returns the
  page's text, so keep your prose short.
- A `spec` is kept in memory as a view (`oxplow:oxplow/view?id=…`) until
  oxplow restarts. The person can **save it to the wiki** (as an
  `oxplow-page` block) or **as a page** in their extension (default
  `my-pages`), where it becomes a file.
- To save it yourself as a page, write the YAML with
  `oxplow.extension.write_page { extension, page, spec, deploy? }`
  (`dashboards.md`).

Prefer `oxplow_find_page` first: a page that answers the question already
exists more often than not.
