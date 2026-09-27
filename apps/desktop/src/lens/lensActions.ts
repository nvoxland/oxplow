/// A lens's declared buttons (tsk329): `copy`, `add-to-context`,
/// `run-source`, from a fixed registry. `copy` and `run-source` go through
/// the backend (the same code MCP's `run_lens_action` runs);
/// `add-to-context` is local. A `run-source` never approves an exec source:
/// that consent is given in Settings → Data. See `.context/extensions.md`.
import { formatContextMention } from "../agent-context-ref.js";
import type { LensAction, LensActionResult, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

export interface LensActionDeps {
  runLensAction(id: string, action: string, params: Record<string, SqlCell>, streamId: string | null): Promise<LensActionResult>;
  copyText(text: string): Promise<void>;
  insertIntoAgent(text: string): void;
  toast(message: string): void;
  recordError(label: string, message: string): void;
}

export async function performLensAction(
  action: LensAction,
  run: LensRun,
  streamId: string | null,
  deps: LensActionDeps,
): Promise<"done" | "failed"> {
  if (action.kind === "add-to-context") {
    deps.insertIntoAgent(formatContextMention({ kind: "lens", lensId: run.lens.id, params: run.params }));
    return "done";
  }
  try {
    const out = await deps.runLensAction(run.lens.id, action.id, run.params, streamId);
    if (action.kind === "copy") {
      await deps.copyText(out.text ?? "");
    } else if (out.report) {
      const counts = Object.entries(out.report.rowCounts)
        .map(([entity, n]) => `${n} ${entity}`)
        .join(", ");
      deps.toast(`Synced ${action.source}: ${counts || "no rows"}.`);
    }
    return "done";
  } catch (e) {
    deps.recordError(action.label, String(e));
    return "failed";
  }
}
