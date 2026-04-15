import { mkdirSync, readdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { randomBytes } from "node:crypto";

export type PaneKind = "working" | "talking";
export type StreamBranchSource = "local" | "remote" | "new";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  branch_ref: string;
  branch_source: StreamBranchSource;
  worktree_path: string;
  created_at: string;
  updated_at: string;
  panes: {
    working: string;
    talking: string;
  };
  resume: {
    working_session_id: string;
    talking_session_id: string;
  };
}

export class StreamStore {
  private projectDir: string;
  private rootDir: string;
  private dir: string;
  private statePath: string;
  private streams = new Map<string, Stream>();

  constructor(projectDir: string) {
    this.projectDir = projectDir;
    this.rootDir = join(projectDir, ".newde");
    this.dir = join(this.rootDir, "streams");
    this.statePath = join(this.rootDir, "state.json");
    mkdirSync(this.rootDir, { recursive: true });
    mkdirSync(this.dir, { recursive: true });
    this.loadAll();
  }

  private loadAll() {
    if (!existsSync(this.dir)) return;
    for (const f of readdirSync(this.dir)) {
      if (!f.endsWith(".yml")) continue;
      try {
        const s = withDefaults(parseYaml(readFileSync(join(this.dir, f), "utf8")), this.projectDir);
        if (s.id) this.streams.set(s.id, s);
      } catch (e) {
        console.warn(`[stream-store] failed to parse ${f}:`, e);
      }
    }
  }

  list(): Stream[] {
    return [...this.streams.values()].sort((a, b) => a.created_at.localeCompare(b.created_at));
  }

  get(id: string): Stream | undefined {
    return this.streams.get(id);
  }

  getCurrentStreamId(): string | null {
    if (!existsSync(this.statePath)) return null;
    try {
      const state = JSON.parse(readFileSync(this.statePath, "utf8"));
      return typeof state.currentStreamId === "string" ? state.currentStreamId : null;
    } catch {
      return null;
    }
  }

  getCurrent(): Stream | undefined {
    const id = this.getCurrentStreamId();
    return id ? this.get(id) : undefined;
  }

  setCurrentStreamId(id: string) {
    if (!this.streams.has(id)) {
      throw new Error(`unknown stream: ${id}`);
    }
    writeFileSync(this.statePath, JSON.stringify({ currentStreamId: id }, null, 2) + "\n", "utf8");
  }

  ensureCurrentStreamId(fallbackId: string) {
    const existing = this.getCurrentStreamId();
    if (existing && this.streams.has(existing)) return existing;
    this.setCurrentStreamId(fallbackId);
    return fallbackId;
  }

  findByBranch(branch: string): Stream | undefined {
    return this.list().find((s) => s.branch === branch);
  }

  findByPane(target: string): { stream: Stream; pane: PaneKind } | undefined {
    for (const stream of this.list()) {
      if (stream.panes.working === target) return { stream, pane: "working" };
      if (stream.panes.talking === target) return { stream, pane: "talking" };
    }
    return undefined;
  }

  create(input: {
    title: string;
    summary?: string;
    branch: string;
    branchRef?: string;
    branchSource?: StreamBranchSource;
    worktreePath: string;
    projectBase: string;
  }): Stream {
    const id = "s-" + randomBytes(4).toString("hex");
    const now = new Date().toISOString();
    const stream: Stream = {
      id,
      title: input.title,
      summary: input.summary ?? "",
      branch: input.branch,
      branch_ref: input.branchRef ?? `refs/heads/${input.branch}`,
      branch_source: input.branchSource ?? "local",
      worktree_path: input.worktreePath,
      created_at: now,
      updated_at: now,
      panes: {
        working: `newde-${input.projectBase}:working-${id}`,
        talking: `newde-${input.projectBase}:talking-${id}`,
      },
      resume: {
        working_session_id: "",
        talking_session_id: "",
      },
    };
    this.save(stream);
    return stream;
  }

  save(stream: Stream) {
    const normalized = withDefaults(stream, this.projectDir);
    this.streams.set(normalized.id, normalized);
    writeFileSync(join(this.dir, `${normalized.id}.yml`), formatYaml(normalized));
  }

  update(streamId: string, mutate: (stream: Stream) => Stream): Stream {
    const existing = this.get(streamId);
    if (!existing) throw new Error(`unknown stream: ${streamId}`);
    const updated = mutate(existing);
    updated.updated_at = new Date().toISOString();
    this.save(updated);
    return updated;
  }
}

function formatYaml(s: Stream): string {
  const esc = (v: string) => JSON.stringify(v);
  return [
    `id: ${esc(s.id)}`,
    `title: ${esc(s.title)}`,
    `summary: ${esc(s.summary)}`,
    `branch: ${esc(s.branch)}`,
    `branch_ref: ${esc(s.branch_ref)}`,
    `branch_source: ${esc(s.branch_source)}`,
    `worktree_path: ${esc(s.worktree_path)}`,
    `created_at: ${esc(s.created_at)}`,
    `updated_at: ${esc(s.updated_at)}`,
    `panes:`,
    `  working: ${esc(s.panes.working)}`,
    `  talking: ${esc(s.panes.talking)}`,
    `resume:`,
    `  working_session_id: ${esc(s.resume.working_session_id)}`,
    `  talking_session_id: ${esc(s.resume.talking_session_id)}`,
    ``,
  ].join("\n");
}

function parseYaml(text: string): Stream {
  const out: any = { panes: {}, resume: {} };
  const lines = text.split("\n");
  let section: "panes" | "resume" | null = null;
  for (const line of lines) {
    if (!line.trim() || line.trim().startsWith("#")) continue;
    const sectionMatch = /^([a-z_]+):\s*$/.exec(line);
    if (sectionMatch) {
      const [, key] = sectionMatch;
      section = key === "panes" || key === "resume" ? key : null;
      if (section) out[section] = out[section] ?? {};
      continue;
    }
    const m = /^(\s*)([a-z_]+):\s*(.*)$/.exec(line);
    if (!m) continue;
    const [, indent, key, rawVal] = m;
    const val = rawVal.trim();
    if (!val) continue;
    const parsed = val.startsWith('"') ? JSON.parse(val) : val;
    if (indent.length > 0) {
      if (section) out[section][key] = parsed;
    } else {
      out[key] = parsed;
    }
  }
  return out as Stream;
}

function withDefaults(stream: Partial<Stream>, projectDir: string): Stream {
  return {
    id: stream.id ?? "",
    title: stream.title ?? stream.branch ?? "",
    summary: stream.summary ?? "",
    branch: stream.branch ?? "",
    branch_ref: stream.branch_ref ?? (stream.branch ? `refs/heads/${stream.branch}` : ""),
    branch_source: stream.branch_source ?? "local",
    worktree_path: stream.worktree_path ?? projectDir,
    created_at: stream.created_at ?? new Date(0).toISOString(),
    updated_at: stream.updated_at ?? stream.created_at ?? new Date(0).toISOString(),
    panes: {
      working: stream.panes?.working ?? "",
      talking: stream.panes?.talking ?? "",
    },
    resume: {
      working_session_id: stream.resume?.working_session_id ?? "",
      talking_session_id: stream.resume?.talking_session_id ?? "",
    },
  };
}
