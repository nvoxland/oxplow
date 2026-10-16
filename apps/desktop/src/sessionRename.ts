/// Renaming an agent session. Rename sets oxplow's alias for the slot (the
/// tab's name once set); the harness's own name for the conversation is
/// separate, shown when there is no alias, and renamed only where the
/// harness can (feature `native_rename`) and the person asks.

/** What the Rename strip says and offers. */
export function renameHint(session: {
  title: string;
  nativeTitle: string;
  displayName: string;
  harnessTitle: string;
  nativeRename: boolean;
}): { message: string; placeholder: string; toggleLabel: string | null } {
  // What the name falls back to once the alias is cleared: the harness's
  // name for the conversation, else the view's own fallback.
  const fallback = session.nativeTitle || (session.title ? "" : session.displayName);
  return {
    message: `This sets oxplow's name for this session. Clear it to use ${
      session.nativeTitle ? session.nativeTitle : `${session.harnessTitle}'s`
    }${session.nativeTitle ? "" : " name for it"}.`,
    placeholder: fallback || session.displayName,
    toggleLabel: session.nativeRename ? `Also rename it in ${session.harnessTitle}` : null,
  };
}

/** Rename session `alias` (empty clears it), then, when asked and there is
 *  a name, the harness's own. A native failure after the alias held says
 *  so: the alias is saved and the harness didn't take the name. */
export async function renameSession(args: {
  alias: string;
  native: boolean;
  harnessTitle: string;
  setAlias(alias: string): Promise<void>;
  setNative(name: string): Promise<void>;
}): Promise<void> {
  await args.setAlias(args.alias);
  if (!args.native || !args.alias) return;
  try {
    await args.setNative(args.alias);
  } catch (e) {
    const why = e instanceof Error ? e.message : String(e);
    throw new Error(`Saved oxplow's name, but ${args.harnessTitle} didn't take it: ${why}`);
  }
}
