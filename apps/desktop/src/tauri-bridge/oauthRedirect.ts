// A sign-in's redirect, caught by the desktop shell (P10,
// `.context/providers.md` → "Signing in").
//
// The service sends the person's browser back to a loopback port on their
// own machine, so the shell — never the core, which may be a remote
// daemon — listens there. The renderer hands each redirect it catches to
// the core (`completeOauthSignIn`) and answers the browser with the
// core's verdict. A plain-browser session has no shell, so it can't sign
// in at all.

import { commands, type SignInCompletion } from "./generated/bindings";

function tauriHostAvailable(): boolean {
  try {
    return "__TAURI_INTERNALS__" in window;
  } catch {
    return false;
  }
}

function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  const error = result.error as { message?: unknown } | null;
  throw new Error(typeof error?.message === "string" ? error.message : String(result.error));
}

/// Whether this window can catch a sign-in's redirect: only the desktop
/// app can.
export function canCatchSignInRedirect(): boolean {
  return tauriHostAvailable();
}

/// Listen for a sign-in's redirect on `port` (a service with its port
/// registered), or any free port: the port.
export async function listenForSignInRedirect(port: number | null): Promise<number> {
  return unwrap(await commands.listenForOauthRedirect(port));
}

/// The next redirect to `port` (its path and query); rejects when the
/// sign-in is stopped or not finished in time.
export async function awaitSignInRedirect(port: number): Promise<string> {
  return unwrap(await commands.awaitOauthRedirect(port));
}

/// Answer the browser with the core's verdict on its redirect; the
/// listener stops unless the redirect wasn't the sign-in's.
export async function answerSignInRedirect(port: number, outcome: SignInCompletion): Promise<void> {
  unwrap(await commands.answerOauthRedirect(port, outcome));
}
