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

/// A listener the shell opened for a sign-in: its id (what waits on,
/// answers and stops it — never its port, which a newer sign-in may
/// reuse) and the port the service sends the browser back to.
export type SignInListener = { id: number; port: number };

/// Listen for a sign-in's redirect on `port` (a service with its port
/// registered), or any free port.
export async function listenForSignInRedirect(port: number | null): Promise<SignInListener> {
  return unwrap(await commands.listenForOauthRedirect(port));
}

/// The next redirect to listener `id` (its path and query); rejects when
/// the sign-in is stopped or not finished in time.
export async function awaitSignInRedirect(id: number): Promise<string> {
  return unwrap(await commands.awaitOauthRedirect(id));
}

/// Answer the browser with the core's verdict on its redirect; the
/// listener stops unless the redirect wasn't the sign-in's.
export async function answerSignInRedirect(id: number, outcome: SignInCompletion): Promise<void> {
  unwrap(await commands.answerOauthRedirect(id, outcome));
}

/// Stop listener `id`: resolves once its socket is closed, so a new
/// sign-in can listen on the same port.
export async function stopSignInRedirect(id: number): Promise<void> {
  unwrap(await commands.stopOauthRedirect(id));
}
