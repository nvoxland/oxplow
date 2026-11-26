import type { ReactNode } from "react";

import type { Stream, Thread, ThreadMark, ThreadState } from "../api.js";
import type { AgentStatusDotState } from "../components/AgentStatusDot.js";
import type { TabRef } from "../tabs/tabState.js";
import type { WindowTab } from "../window/windowTab.js";

/**
 * What a window gives the shell that draws it: its streams and threads, the
 * shown thread's tabs, and what a shell does with them. The window logic
 * (`App` and the `window/` hooks) makes it once; the desktop layout and the
 * tablet shell (`TabletShell`) each draw it their way. A shell draws every
 * contribution the window has (`shellContract.ts`).
 */
export interface ShellWindow {
  streams: Stream[];
  stream: Stream | null;
  threadStates: Record<string, ThreadState>;
  selectedThread: Thread | null;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  agentQuestions: Record<string, string | undefined>;
  switchStream(id: string): void | Promise<void>;
  selectThread(streamId: string, threadId: string): void | Promise<void>;
  /** Whether it was made (a failure is the runner's: recorded, toasted). */
  createThread(streamId: string, title: string): Promise<boolean>;
  renameStream(streamId: string, title: string): void | Promise<void>;
  renameThread(threadId: string, title: string): void | Promise<void>;
  markThread(threadId: string, mark: ThreadMark | null): void | Promise<void>;
  closeThread(threadId: string): void | Promise<void>;
  openStreamSettings(streamId: string): void;
  openThreadSettings(threadId: string): void;
  openNewStream(): void;
  /** The shown thread's tabs, agent sessions first. */
  tabs: WindowTab[];
  activeTabId: string | null;
  activateTab(id: string): void;
  closeTab(id: string): void;
  openPage(ref: TabRef): void;
  /** Open Quick Open (search, Go To, the commands). */
  openSearch(): void;
  /** The window's panels: the dock today (`Dock`), drawn where the shell
   *  keeps them. */
  tray: ReactNode;
}
