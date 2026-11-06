import { execFileSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "../../support/fixtures.js";
import { ipc, run, settle, startDaemon, until, type Daemon } from "../../support/daemon.js";

// What a real watch on a real daemon has to deliver: a change made outside
// oxplow reaches what follows it, with no restart. The logic behind it is tested
// with no OS (`oxplow-fs-watch`); this is the one place all of it runs together.
// On a daemon of its own: it makes a second stream, and edits the config.

type Rows = { rows: unknown[][] };
type Stream = { id: string; title: string; worktree_path: string };

const count = async (daemon: Daemon, sql: string): Promise<number> => {
  const out = await ipc<Rows>(daemon, "query_sql", { sql });
  return Number(out.rows[0]?.[0] ?? 0);
};

/** The working-tree list of a stream holds `path`. */
const listed = (daemon: Daemon, stream: string, path: string) =>
  until(`${path} in ${stream}'s working files`, 30_000, async () => {
    const n = await count(
      daemon,
      `SELECT count(*) FROM v_change_file f JOIN v_change c ON c.id = f.change_id
       WHERE c.kind = 'working' AND c.stream_id = ${stream.replace(/^str/, "")} AND f.path = '${path}'`,
    );
    return n === 1;
  });

/** A stream's snapshots hold `path`. A take comes once the tree has been quiet. */
const snapshotted = (daemon: Daemon, path: string) =>
  until(`${path} in a snapshot`, 30_000, async () => {
    return (await count(daemon, `SELECT count(*) FROM v_snapshot_file WHERE path = '${path}'`)) > 0;
  });

test("what changes on disk reaches what follows it, with no restart", async () => {
  test.setTimeout(240_000);
  const daemon = await startDaemon();
  try {
    await settle(daemon);
    const [primary] = await ipc<Stream[]>(daemon, "list_streams");
    if (!primary) throw new Error("no stream");

    // An edit reaches the working-files list.
    writeFileSync(join(daemon.project, "edited.txt"), "hop\n");
    await listed(daemon, primary.id, "edited.txt");

    // A folder made after the watch started: `mkdir d && write d/f` races any
    // new watch, and the file is in a snapshot all the same.
    mkdirSync(join(daemon.project, "made-later/deep"), { recursive: true });
    writeFileSync(join(daemon.project, "made-later/deep/f.txt"), "x\n");
    await snapshotted(daemon, "made-later/deep/f.txt");

    // A burst larger than anything could queue path by path: all of it lands.
    mkdirSync(join(daemon.project, "burst"));
    for (let i = 0; i < 3000; i += 1) writeFileSync(join(daemon.project, `burst/f${i}.txt`), `${i}\n`);
    await until("the whole burst in the working files", 60_000, async () => {
      const n = await count(
        daemon,
        `SELECT count(*) FROM v_change_file f JOIN v_change c ON c.id = f.change_id
         WHERE c.kind = 'working' AND c.stream_id = ${primary.id.replace(/^str/, "")} AND f.path LIKE 'burst/%'`,
      );
      return n === 3000;
    });

    // The wiki's row follows its file.
    writeFileSync(join(daemon.project, ".oxplow/wiki/by-hand.md"), "# By hand\n\nbody\n");
    await until("the wiki page's row", 30_000, async () => {
      return (await count(daemon, "SELECT count(*) FROM v_knowledge_page WHERE slug = 'by-hand'")) === 1;
    });

    // A hand edit of the config goes live, and a `generated` change applies:
    // what it excludes leaves the file list, and goes out of the next snapshot.
    const leftOut = () => count(daemon, "SELECT count(*) FROM v_file WHERE path = 'left-out/before.txt'");
    mkdirSync(join(daemon.project, "left-out"));
    writeFileSync(join(daemon.project, "left-out/before.txt"), "x\n");
    await until("the folder in the file list", 30_000, async () => (await leftOut()) === 1);
    appendFileSync(join(daemon.project, ".oxplow/project.yaml"), "\ngenerated:\n  exclude:\n    - left-out\n");
    await until("the hand edit of the config to apply", 30_000, async () => (await leftOut()) === 0);
    writeFileSync(join(daemon.project, "left-out/no.txt"), "no\n");
    writeFileSync(join(daemon.project, "after-generated.txt"), "yes\n");
    await snapshotted(daemon, "after-generated.txt");
    expect(await count(daemon, "SELECT count(*) FROM v_snapshot_file WHERE path = 'left-out/no.txt'")).toBe(0);

    // A stream made now is followed like one that was there at boot: its
    // files, by its own ignore rules, and its commits.
    await run(daemon, "oxplow.stream.create", {
      title: "Made Later",
      source: "new",
      new_branch: "made-later",
      start_point: "HEAD",
    });
    const streams = await ipc<Stream[]>(daemon, "list_streams");
    const later = streams.find((s) => s.title === "Made Later");
    if (!later) throw new Error(`no new stream: ${JSON.stringify(streams)}`);
    const tree = later.worktree_path;
    writeFileSync(join(tree, ".gitignore"), `${readFileSyncOr(join(tree, ".gitignore"))}only-here.txt\n`);
    writeFileSync(join(tree, "only-here.txt"), "ignored here\n");
    writeFileSync(join(tree, "in-the-worktree.txt"), "x\n");
    await listed(daemon, later.id, "in-the-worktree.txt");
    await snapshotted(daemon, "in-the-worktree.txt");
    expect(
      await count(daemon, "SELECT count(*) FROM v_snapshot_file WHERE path = 'only-here.txt'"),
      "its own .gitignore rules it",
    ).toBe(0);
    execFileSync("git", ["add", "in-the-worktree.txt"], { cwd: tree });
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@e", "commit", "-m", "Commit in the later stream"], {
      cwd: tree,
    });
    await until("the later stream's commit", 30_000, async () => {
      return (await count(daemon, "SELECT count(*) FROM v_commit WHERE subject LIKE 'Commit in the later stream%'")) > 0;
    });

    // Nothing here overflowed anything on its way to a subscriber.
    expect(readFileSync(daemon.log, "utf8")).not.toContain("watcher lagged");
  } finally {
    await daemon.stop();
  }
});

function readFileSyncOr(path: string): string {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "";
  }
}
