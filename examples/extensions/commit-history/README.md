# commit-history

A page of the project's commits, made only of lens files. It shows the
param types and a row that selects:

- The toolbar has a `date_range` (`When`), a `search` (`Search subjects`)
  and a `bool` (`Merges only`). The commits query reads the range as
  `:range_from` and `:range_to`.
- The `Commit` column has `select: commit`. Clicking a row sets the
  page's `commit` param, and the "Files in the commit" section runs again
  with it.

Copy the folder into your project's `.oxplow/extensions/`, deploy it
(`oxplow.extension.deploy { name: commit-history }`), then open "Commit
history" from the launcher.
