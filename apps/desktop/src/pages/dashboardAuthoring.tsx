/** The same page authoring path for visual edits and pinning. Tests
 * provide a backend without replacing process-wide API modules. */
import { createContext, useContext } from "react";
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
