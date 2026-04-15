import { mkdirSync, readdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { randomBytes } from "node:crypto";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  created_at: string;
  updated_at: string;
  panes: {
    working: string;
    talking: string;
  };
}

export class StreamStore {
  private dir: string;
  private streams = new Map<string, Stream>();

  constructor(projectDir: string) {
    this.dir = join(projectDir, ".newde", "streams");
    mkdirSync(this.dir, { recursive: true });
    this.loadAll();
  }

  private loadAll() {
    if (!existsSync(this.dir)) return;
    for (const f of readdirSync(this.dir)) {
      if (!f.endsWith(".yml")) continue;
      try {
        const s = parseYaml(readFileSync(join(this.dir, f), "utf8"));
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

  findByBranch(branch: string): Stream | undefined {
    return this.list().find((s) => s.branch === branch);
  }

  create(input: { title: string; summary?: string; branch: string; projectBase: string }): Stream {
    const id = "s-" + randomBytes(4).toString("hex");
    const now = new Date().toISOString();
    const stream: Stream = {
      id,
      title: input.title,
      summary: input.summary ?? "",
      branch: input.branch,
      created_at: now,
      updated_at: now,
      panes: {
        working: `newde-${input.projectBase}:working-${id}`,
        talking: `newde-${input.projectBase}:talking-${id}`,
      },
    };
    this.save(stream);
    return stream;
  }

  save(stream: Stream) {
    this.streams.set(stream.id, stream);
    writeFileSync(join(this.dir, `${stream.id}.yml`), formatYaml(stream));
  }
}

function formatYaml(s: Stream): string {
  const esc = (v: string) => JSON.stringify(v);
  return [
    `id: ${esc(s.id)}`,
    `title: ${esc(s.title)}`,
    `summary: ${esc(s.summary)}`,
    `branch: ${esc(s.branch)}`,
    `created_at: ${esc(s.created_at)}`,
    `updated_at: ${esc(s.updated_at)}`,
    `panes:`,
    `  working: ${esc(s.panes.working)}`,
    `  talking: ${esc(s.panes.talking)}`,
    ``,
  ].join("\n");
}

function parseYaml(text: string): Stream {
  const out: any = { panes: {} };
  const lines = text.split("\n");
  for (const line of lines) {
    if (!line.trim() || line.trim().startsWith("#")) continue;
    const m = /^(\s*)([a-z_]+):\s*(.*)$/.exec(line);
    if (!m) continue;
    const [, indent, key, rawVal] = m;
    const val = rawVal.trim();
    if (!val) continue;
    const parsed = val.startsWith('"') ? JSON.parse(val) : val;
    if (indent.length > 0) {
      out.panes[key] = parsed;
    } else {
      out[key] = parsed;
    }
  }
  return out as Stream;
}
