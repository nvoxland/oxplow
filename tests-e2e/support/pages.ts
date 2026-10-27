// Extensions a spec writes for itself: a manifest, pages and any other file,
// deployed as a person's own would be.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

import { deploy, extensionSource, type Daemon } from "./daemon.js";

/** Write extension `name` into `daemon`'s project and deploy it. `manifest`
 *  is YAML added under the header (`ui:`, `panels:` …); `files` are paths in
 *  the extension (`pages/x.yaml`, `components/y/index.html`) with their text. */
export async function installExtension(
  daemon: Daemon,
  name: string,
  { manifest = "", files }: { manifest?: string; files: Record<string, string> },
): Promise<void> {
  const dir = extensionSource(daemon, name);
  const head = `manifest: 2\nname: ${name}\nintent:\n  purpose: A page behaviour the browser suite checks.\n  origin: null\n  examples: []\n`;
  const all = { "extension.yaml": head + manifest, ...files };
  for (const [path, text] of Object.entries(all)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  }
  await deploy(daemon, name);
}
