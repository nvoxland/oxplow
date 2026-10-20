# commit-history

A page of the project's commits, made only of lens files. It shows the
param types and a row that selects:

- Its filters are a `date_range` (`When`), a `search` (`Search subjects`)
  and a `bool` (`Merges only`). The page's rail shows them, each beside
  its value; click a value to change it. The commits query reads the
  range as `:range_from` and `:range_to`.
- The `Commit` column has `select: commit`, so `commit` is the page's
  selection: the rail shows the picked commit, not an input. Clicking a
  row sets it, and the "Files in the commit" section runs again with it.

Copy the folder into your project's `.oxplow/extensions/`, deploy it
(`oxplow.extension.deploy { name: commit-history }`), then open "Commit
history" from the launcher.
