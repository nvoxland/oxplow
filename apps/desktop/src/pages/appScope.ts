import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Origin } from "../tabs/session.js";

/** Navigation bound to one tab slot: in-tab navigation mutates the slot. */
export type SlotNav = {
  /** The slot's current ref id. */
  slotId: string;
  navOpen: (newRef: TabRef, opts?: { newTab?: boolean; origin?: Origin }) => void;
  navOpenFile: (path: string, opts?: { newTab?: boolean }) => void;
  navOpenDiff: (spec: DiffSpec, origin?: Origin) => void;
};
