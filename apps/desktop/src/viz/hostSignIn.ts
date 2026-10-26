/// Signing in for a provider instance's credential, run by the host for a
/// component page (`signIn` over the bridge): the person's own browser
/// opens the service's page, the desktop shell listens for the redirect
/// on this machine, and each redirect it catches goes to the core
/// (`oxplow.provider.complete_sign_in`), the browser hearing the core's
/// verdict. The frame is sandboxed and can do none of this; it asks, and
/// is told how it ended. One sign-in a credential: a newer one replaces
/// the one under way, which ends `cancelled` (`.context/providers.md`
/// "Credentials and sign-in").
import type { SignInCompletion } from "../tauri-bridge/generated/bindings.js";
import type { SignInResult } from "./componentBridge.js";

export interface SignInDeps {
  /** Whether the shell can listen for the redirect (the desktop app). */
  canCatch(): boolean;
  /** The port the credential's service registered for the redirect, if any. */
  redirectPort(instance: string, name: string): Promise<number | null>;
  listen(port: number | null): Promise<{ id: number; port: number }>;
  awaitRedirect(listener: number): Promise<string>;
  answer(listener: number, outcome: SignInCompletion): Promise<void>;
  stop(listener: number): Promise<void>;
  openBrowser(url: string): Promise<void>;
  begin(instance: string, name: string, port: number): Promise<{ url: string; signIn: number }>;
  complete(instance: string, name: string, redirect: string): Promise<SignInCompletion>;
  cancel(instance: string, name: string, signIn: number): Promise<void>;
}

interface UnderWay {
  listener: number | null;
  signIn: number | null;
  /** Set when it was ended from outside: its wait resolves `cancelled`. */
  ended: boolean;
}

export function createSignInHost(deps: SignInDeps) {
  const underWay = new Map<string, UnderWay>();
  const key = (instance: string, name: string) => `${instance}\u0000${name}`;
  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  /** End `s`: the shell stops listening (resolved once its socket is
   *  closed) and the core forgets the sign-in. */
  async function end(s: UnderWay, instance: string, name: string) {
    s.ended = true;
    const { listener, signIn } = s;
    s.listener = null;
    s.signIn = null;
    if (listener !== null) await deps.stop(listener).catch(() => {});
    if (signIn !== null) await deps.cancel(instance, name, signIn).catch(() => {});
  }

  async function cancel(instance: string, name: string): Promise<void> {
    const k = key(instance, name);
    const s = underWay.get(k);
    if (!s) return;
    underWay.delete(k);
    await end(s, instance, name);
  }

  async function signIn(instance: string, name: string): Promise<SignInResult> {
    if (!deps.canCatch()) {
      throw Object.assign(
        new Error("Signing in needs the oxplow desktop app: the service sends the browser back to this machine, where only the app can listen."),
        { code: "INVALID" },
      );
    }
    // A newer sign-in replaces the one under way — its socket closed
    // before this one listens, on a declared port the same one.
    await cancel(instance, name);
    const mine: UnderWay = { listener: null, signIn: null, ended: false };
    underWay.set(key(instance, name), mine);
    const over = () => {
      if (underWay.get(key(instance, name)) === mine) underWay.delete(key(instance, name));
    };
    let listener: number;
    try {
      const listening = await deps.listen(await deps.redirectPort(instance, name));
      listener = listening.id;
      mine.listener = listener;
      const begun = await deps.begin(instance, name, listening.port);
      mine.signIn = begun.signIn;
      await deps.openBrowser(begun.url);
    } catch (e) {
      over();
      await end(mine, instance, name);
      throw e;
    }
    // Each redirect the shell catches goes to the core, and the browser
    // hears its verdict; one that isn't this sign-in's is refused and the
    // wait goes on.
    try {
      for (;;) {
        const redirect = await deps.awaitRedirect(listener);
        const outcome: SignInCompletion = await deps
          .complete(instance, name, redirect)
          .catch((e: unknown) => ({ outcome: "failed", error: message(e) }) as SignInCompletion);
        await deps.answer(listener, outcome);
        if (outcome.outcome === "not_this_sign_in") continue;
        // Over in the core too: nothing to cancel.
        mine.signIn = null;
        over();
        await end(mine, instance, name);
        return outcome.outcome === "failed" ? { state: "failed", error: outcome.error } : { state: "signed_in" };
      }
    } catch (e) {
      // Stopped by a newer sign-in or a cancel: that is how it ended.
      if (mine.ended) return { state: "cancelled" };
      over();
      await end(mine, instance, name);
      return { state: "failed", error: message(e) };
    }
  }

  /** The frame is gone: every sign-in it had under way ends. */
  async function closeAll(): Promise<void> {
    const all = [...underWay.entries()];
    underWay.clear();
    await Promise.all(
      all.map(([k, s]) => {
        const [instance, name] = k.split("\u0000");
        return end(s, instance, name);
      }),
    );
  }

  return { signIn, cancel, closeAll };
}
