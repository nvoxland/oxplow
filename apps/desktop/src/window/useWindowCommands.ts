import type { Stream, Thread, WorkspaceContext } from "../api.js";
import type { ClientHandlers } from "../clientHost.js";
import { commandOpenAfter, runCommandInBackground, shellAvailable, type GitOpKickoff } from "../api.js";
import { runLocally } from "../clientHost.js";
import { commandOffers, type OfferDeps } from "../commandOffers.js";
import { setRefOfferHost } from "../components/refCommands.js";
import { awaitGitOp, opErrorOf } from "../git-op.js";
import { offerForShortcut } from "../keybindings.js";
import { requestNewThread } from "../new-thread-bus.js";
import { recordOpError } from "../opErrors.js";
import { kindForTabId } from "../pageKinds.js";
import { personCommands, setPersonCommandsOpener } from "../personCommands.js";
import { usePersonCommands } from "../personCommandsStore.js";
import { navigationKey } from "../tabs/navigationKeys.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import { type TabRef } from "../tabs/tabState.js";
import { type CommandOutcome } from "../tauri-bridge/generated/bindings.js";
import { useLiveTerminals } from "../terminal/useTerminalSession.js";
import { useTreeDirty } from "../treeDirty.js";
import { useAgentHarnesses } from "../useAgentHarnesses.js";
import { type WhenContext } from "../when.js";
import { useCallback, useEffect, useMemo } from "react";
export interface WindowCommandsOptions {
  stream: Stream | null;
  selectedThread: Thread | null;
  workspaceContext: WorkspaceContext;
  /** The agent harnesses, for `enabledAgents`. */
  harnesses: ReturnType<typeof useAgentHarnesses>;
  /** The shown tab: what it is and whether it holds unsaved edits. */
  tabs: { centerActive: string; dirtyTabs: Record<string, boolean>; shownFilePath: string | null };
  handleOpenPageRef: { current: ((ref: TabRef) => void) | null };
  /** What the window hosts: a command it can do runs here, not on the daemon. */
  windowHandlersRef: { current: ClientHandlers };
  /** The active tab's Back, Forward and origin steps, read at the key press. */
  navKeysRef: { current: { back: () => void; forward: () => void; step: (by: number) => void; mode: () => void } };
}

/** The window's commands: the context keys a command's `when` reads, what
 *  the command bus offers the person (search, menus, a ref's menus), and the
 *  keyboard shortcuts that run an offer or step through the active tab.
 *  Shell-neutral: a shell draws its own menus from `offers`. */
export function useWindowCommands({
  stream,
  selectedThread,
  workspaceContext,
  harnesses,
  tabs,
  handleOpenPageRef,
  windowHandlersRef,
  navKeysRef,
}: WindowCommandsOptions) {
  const { centerActive, dirtyTabs, shownFilePath } = tabs;
  const selectedThreadId = selectedThread?.id ?? null;
  // The window's context keys: what a command's `when` reads
  // (`oxplow_domain::when::CONTEXT_KEYS`; `.context/commands.md`).
  const treeDirty = useTreeDirty(stream?.id);
  // The agent terminals a pane shows running: Restart Terminal is offered
  // for these only (an ended pane has Start Again).
  const liveTerminals = useLiveTerminals();
  const whenContext = useMemo<WhenContext>(
    () => ({
      streamShown: !!stream,
      streamKind: stream?.kind ?? "",
      threadShown: !!selectedThread,
      fileShown: shownFilePath !== null,
      pageDirty: !!centerActive && !!dirtyTabs[centerActive],
      pageKind: centerActive ? kindForTabId(centerActive) : "",
      agentTerminalLive: liveTerminals.has((centerActive && refFromTabId(centerActive)?.id) || ""),
      vcsEnabled: !!workspaceContext.vcsEnabled,
      treeDirty,
      shellAvailable: shellAvailable(),
      enabledAgents: harnesses
        .filter((h) => h.enabled)
        .map((h) => h.id)
        .join(","),
    }),
    [centerActive, dirtyTabs, harnesses, liveTerminals, selectedThread, shownFilePath, stream, treeDirty, workspaceContext.vcsEnabled],
  );
  // Run a Git-menu mutation (pull/push) as a background task and surface
  // any failure the same way the Git Dashboard does: record an op-error
  // and offer a toast that opens its detail page. There's no page focus
  // here, so we route through handleOpenPageRef rather than onOpenPage.
  const runGitMenuOp = useCallback(
    async (
      label: string,
      command: string,
      action: () => Promise<GitOpKickoff>,
    ) => {
      const result = await awaitGitOp(await action());
      if (result.success) return;
      // Surfaces globally (toast + status-bar indicator) via recordOpError.
      recordOpError(opErrorOf(label, command, result));
    },
    [],
  );
  /** One of the window's own forms — a command's `ui.form` that isn't a
   *  tab id: what gathers New Thread's and Commit's input. */
  const openForm = useCallback((name: string) => {
    switch (name) {
      case "new-thread":
        // The Navigator owns thread creation (inline title + agent picker).
        if (stream) requestNewThread(stream.id);
        return;
      default:
        recordOpError({ label: "Open form", message: `the window has no form \`${name}\`` });
    }
  }, [stream, workspaceContext.vcsEnabled]);

  // A toast's Open link opens a page the way a command offer does.
  useEffect(() => {
    setPersonCommandsOpener((tabId) => {
      const ref = refFromTabId(tabId);
      if (ref) handleOpenPageRef.current?.(ref);
    });
    return () => setPersonCommandsOpener(null);
  }, []);
  // What the command bus offers a person (Pull, New Task, …): search lists
  // them with the app's own commands, and a shortcut may run one.
  const personSpecs = usePersonCommands();
  const offerCtx = useMemo(
    () => ({ streamId: stream?.id ?? null, threadId: selectedThreadId ?? null, when: whenContext }),
    [stream?.id, selectedThreadId, whenContext],
  );
  const offerDeps = useMemo<OfferDeps>(
    () => ({
      openPage: (tabId) => {
        const ref = refFromTabId(tabId);
        if (ref) handleOpenPageRef.current?.(ref);
      },
      openForm,
      run: (label, id, input) => {
        // One the window hosts runs here: nothing goes to the daemon.
        const spec = personSpecs.find((s) => s.id === id);
        const local = spec ? runLocally(windowHandlersRef.current, spec, input) : null;
        if (local) {
          return local
            .then(({ result }): CommandOutcome => ({ result, command_run_id: null, event_id: null, inverse: null }))
            .catch((e: unknown) => {
              recordOpError({ label, message: e instanceof Error ? e.message : String(e) });
              return null;
            });
        }
        return personCommands.run(label, id, input);
      },
      runInBackground: (label, id, input) =>
        void runGitMenuOp(label, id, () => runCommandInBackground(label, id, input)),
      openAfter: (id, result) =>
        commandOpenAfter(id, result).catch((e: unknown) => {
          recordOpError({ label: "Open what it made", message: e instanceof Error ? e.message : String(e) });
          return null;
        }),
      }),
    [personSpecs, runGitMenuOp, openForm],
  );
  const offers = useMemo(() => commandOffers(personSpecs, offerCtx, offerDeps), [personSpecs, offerCtx, offerDeps]);
  // A ref's menus (its page's, its rows') run offers the same way.
  useEffect(() => {
    setRefOfferHost({ ctx: offerCtx, deps: offerDeps });
    return () => setRefOfferHost(null);
  }, [offerCtx, offerDeps]);

  // The launcher (QuickOpen) is the single discovery surface — pages,
  // files, commands, and body search in one box — and has exactly one
  // shortcut: Cmd/Ctrl+P (`oxplow.window.quick_open`'s). The old
  // Cmd+K / Cmd+Shift+F aliases were removed: one door is
  // clearer, and Cmd+P is the established dev quick-open reflex. Monaco
  // doesn't bind Cmd+P, so no capture-phase interception is needed — the
  // menu accelerator opens the launcher even when the editor is focused.

  useEffect(() => {
    // Runs in both Electron and browser modes. In Electron the native
    // menu's accelerator should also fire for the same command, but the
    // handler is idempotent (an offer's run → modal setters are no-ops
    // when the modal is already open) so a double-dispatch is harmless
    // — and not relying on the native menu means Cmd+Shift+N works even
    // when the menu snapshot is momentarily stale at startup.
    function handleKeyDown(event: KeyboardEvent) {
      // A command's `ui.shortcut`; typing in a field keeps it unless it
      // runs while typing (Save, Find, Quick Open do — a user mid-way
      // through a description shouldn't lose it to New Task's form).
      const offer = offerForShortcut(offers, event, isEditableTarget(event.target));
      if (offer) {
        event.preventDefault();
        offer.run();
        return;
      }
      // Back, Forward, and Up / Down the page's origin, on the active tab.
      const key = navigationKey(event, event.target);
      if (!key) return;
      const keys = navKeysRef.current;
      event.preventDefault();
      if (key === "back") keys.back();
      else if (key === "forward") keys.forward();
      else if (key === "mode") keys.mode();
      else keys.step(key === "up" ? -1 : 1);
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [offers]);

  return { whenContext, liveTerminals, personSpecs, offerCtx, offerDeps, offers };
}

/** Whether a key press lands in a field that takes typed text: a shortcut
 *  that doesn't run while typing leaves it alone. */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    // Checkbox / button / radio inputs shouldn't block the shortcut — they
    // don't swallow typed characters the way a text field does.
    return type === "text" || type === "search" || type === "email" || type === "url" || type === "password" || type === "" || type === "tel";
  }
  return false;
}
