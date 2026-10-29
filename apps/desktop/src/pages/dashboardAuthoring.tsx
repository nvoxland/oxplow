/** The same page authoring path for visual edits and pinning. Tests
 * provide a backend without replacing process-wide API modules. */
import { createContext, useContext, useEffect, useState } from "react";
import { runCommand, unwrapResult } from "../api.js";
import { commands, type PageSpec } from "../tauri-bridge/generated/bindings.js";
import { recordOpError } from "../opErrors.js";

export interface DashboardWrite { extension: string; page: string; spec: string; deploy: true }
export interface DashboardAuthoring {
  read(page: string): Promise<PageSpec>;
  write(input: DashboardWrite): Promise<unknown>;
  report(error: { label: string; command?: string; message: string }): void;
}
export const DashboardEditContext = createContext<DashboardAuthoring>({
  read: async (page) => unwrapResult(await commands.pageSpec(page)),
  write: (input) => runCommand("oxplow.extension.write_page", input),
  report: recordOpError,
});
export const useDashboardAuthoring = () => useContext(DashboardEditContext);

/** Whether a dashboard's source can be edited here (false until read, and for no page). */
export function useDashboardWritable(page: string | null): boolean {
  const authoring = useDashboardAuthoring();
  const [writable, setWritable] = useState(false);
  useEffect(() => {
    let live = true;
    setWritable(false);
    if (page) authoring.read(page).then((spec) => { if (live) setWritable(spec.writable); }).catch(() => {});
    return () => { live = false; };
  }, [page, authoring]);
  return writable;
}
