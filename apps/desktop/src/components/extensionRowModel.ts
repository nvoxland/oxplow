/** Pure row presentation for the Settings → Extensions list. */
import type { Extension } from "../tauri-bridge/generated/bindings.js";

export interface ExtensionRowModel {
  name: string;
  description: string;
  lensCount: number;
  /** Where it came from: "In this repo", or `<url> @ <ref> (<sha7>)`. */
  origin: string;
  /** Only git-installed extensions can be updated from their source. */
  canUpdate: boolean;
  healthy: boolean;
  errors: string[];
}

export function extensionRowModel(ext: Extension): ExtensionRowModel {
  const src = ext.source;
  const origin = src
    ? `${src.git}${src.gitRef ? ` @ ${src.gitRef}` : ""} (${src.sha.slice(0, 7)})`
    : "In this repo";
  return {
    name: ext.name,
    description: ext.description,
    lensCount: ext.lenses.length,
    origin,
    canUpdate: src !== null,
    healthy: ext.errors.length === 0,
    errors: ext.errors,
  };
}
