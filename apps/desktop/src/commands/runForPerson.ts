/// The one way a person's command runs (`.context/usability.md` "A person's
/// command that fails"): every page action, form, component invoke,
/// proposal decision and core control runs through `run`, which tells what
/// a failure was and shows it the one way:
/// - invalid input naming a field: the caller shows it under the field;
///   not recorded;
/// - the command asks first: the caller asks (`runAsking` runs it again
///   confirmed); not recorded;
/// - cancelled: nothing;
/// - anything else: recorded once (`v_op_error`: the bell, the Alerts page),
///   shown beside the control that ran it (`anchor`), and toasted only when
///   no such control is on screen.
/// It never throws, and never toasts a success (callers keep their Undo).
import { fieldErrors, ipcErrorDetails, needsConfirmation } from "../ipc-error.js";
import { reportOpError, type OpErrorInput } from "../opErrors.js";
import { failureStore, inlineClaims, type FailureStore, type InlineClaims } from "./actionFailures.js";

export type RunFailure =
  | { kind: "field"; errors: Record<string, string> }
  | { kind: "confirm"; details: string[] }
  | { kind: "cancelled" }
  | { kind: "failed"; message: string; code: string | null };

export type RunResult<T> = { ok: true; value: T } | { ok: false; failure: RunFailure };

export interface RunOptions {
  /** The control's key (`useActionAnchor`): a failure shows beside it. */
  anchor?: string;
  /** The command it ran, for the record. */
  command?: string;
  /** The thread it ran for; absent, the one on screen. */
  threadId?: string | null;
}

export interface RunnerDeps {
  report(entry: OpErrorInput): Promise<string | null>;
  failures: FailureStore;
  claims: InlineClaims;
  now(): number;
}

function codeOf(e: unknown): string | null {
  if (e && typeof e === "object") {
    const code = (e as { code?: unknown }).code;
    if (typeof code === "string" && code) return code;
  }
  return null;
}

function messageOf(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object") {
    const m = (e as { message?: unknown }).message;
    if (typeof m === "string") return m;
  }
  return String(e);
}

/** What a failed run came to. A component's rejection is a plain
 *  `{ code, message, field? }`, read the same way. */
export function classifyFailure(e: unknown): RunFailure {
  const fields = fieldErrors(e);
  if (fields) return { kind: "field", errors: fields };
  if (needsConfirmation(e)) return { kind: "confirm", details: ipcErrorDetails(e) };
  const code = codeOf(e);
  if (code === "CANCELLED") return { kind: "cancelled" };
  return { kind: "failed", message: messageOf(e), code };
}

export function createRunner(deps: RunnerDeps) {
  const run = async <T>(label: string, call: () => Promise<T>, opts: RunOptions = {}): Promise<RunResult<T>> => {
    const { anchor } = opts;
    if (anchor) deps.failures.clear(anchor);
    try {
      return { ok: true, value: await call() };
    } catch (e) {
      const failure = classifyFailure(e);
      if (failure.kind !== "failed") return { ok: false, failure };
      const report = deps.report({
        label,
        message: failure.message,
        ...(opts.command ? { command: opts.command } : {}),
        ...(opts.threadId !== undefined ? { threadId: opts.threadId } : {}),
      });
      if (anchor) {
        const at = deps.now();
        deps.failures.set(anchor, { message: failure.message, code: failure.code, errorId: null, at });
        void report.then((errorId) => {
          const now = deps.failures.get(anchor);
          if (errorId && now?.at === at) deps.failures.set(anchor, { ...now, errorId });
        });
        if (deps.failures.isMounted(anchor)) deps.claims.claim(report, () => deps.failures.isMounted(anchor));
      }
      return { ok: false, failure };
    }
  };
  /** `run`, asking the person first when the command asks: `ask` gets what
   *  it asks about and resolves true to run it again confirmed. */
  const runAsking = async <T>(
    label: string,
    call: (confirmed: boolean) => Promise<T>,
    ask: (details: string[]) => Promise<boolean>,
    opts: RunOptions = {},
  ): Promise<RunResult<T>> => {
    const first = await run(label, () => call(false), opts);
    if (first.ok === true) return first;
    const failure = first.failure;
    if (failure.kind !== "confirm") return first;
    if (!(await ask(failure.details))) return { ok: false, failure: { kind: "cancelled" } };
    return run(label, () => call(true), opts);
  };
  return { run, runAsking, failures: deps.failures };
}

export type Runner = ReturnType<typeof createRunner>;

/** The window's runner. */
export const runner: Runner = createRunner({ report: reportOpError, failures: failureStore, claims: inlineClaims, now: Date.now });
export const runForPerson = runner.run;
