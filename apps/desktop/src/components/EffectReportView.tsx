/// What an extension change does (P8.C7): the server's lines
/// (`extension_effects::summary` — grants first, then collectors' outputs,
/// models and their rows, lenses, config), then each changed lens's text
/// before and after. The install review and an effort's review show it.
import type { EffectReport } from "../tauri-bridge/generated/bindings.js";
import { EffectDiff } from "./EffectDiff.js";

/** The lenses whose text changes, before and after. */
export function lensDiffs(report: EffectReport): { id: string; before: string; after: string }[] {
  return report.lenses
    .filter((l) => l.change === "changed" && l.before !== null && l.after !== null)
    .map((l) => ({ id: l.id, before: l.before!, after: l.after! }));
}

export function EffectReportView({ report, testId }: { report: EffectReport; testId: string }) {
  return (
    <>
      {report.lines.length > 0 ? (
        <>
          <div style={{ fontWeight: 600, marginTop: 6 }}>What it changes</div>
          <ul data-testid={testId} style={{ margin: "4px 0", paddingLeft: 18 }}>
            {report.lines.map((line, i) => (
              <li key={i}>{line}</li>
            ))}
          </ul>
        </>
      ) : null}
      {lensDiffs(report).map((d) => (
        <EffectDiff key={d.id} id={d.id} before={d.before} after={d.after} />
      ))}
    </>
  );
}
