/// What an extension change does: the server's lines
/// (`extension_impact::summary` — grants first, then collectors' outputs,
/// models and their rows, lenses), then each changed lens's text
/// before and after. The install review and an effort's review show it.
import type { ImpactReport } from "../tauri-bridge/generated/bindings.js";
import { LensDiff } from "./LensDiff.js";

/** The lenses whose text changes, before and after. */
export function lensDiffs(report: ImpactReport): { id: string; before: string; after: string }[] {
  return report.lenses
    .filter((l) => l.change === "changed" && l.before !== null && l.after !== null)
    .map((l) => ({ id: l.id, before: l.before!, after: l.after! }));
}

export function ImpactReportView({ report, testId }: { report: ImpactReport; testId: string }) {
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
        <LensDiff key={d.id} id={d.id} before={d.before} after={d.after} />
      ))}
    </>
  );
}
