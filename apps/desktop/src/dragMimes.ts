/**
 * The canonical registry of oxplow's **internal** drag-and-drop MIME types.
 *
 * Every in-app drag declares a custom type so a foreign drag — an OS file,
 * a text selection from another app — can never satisfy a drop target's
 * `dataTransfer.types.includes(...)` check. See `.context/usability.md` →
 * "Drag and drop".
 *
 * **A new drag kind gets a new MIME here rather than overloading an
 * existing one**, and it gets it *here* rather than as a `const` next to
 * whichever component happened to introduce it. That was the old shape and
 * it went wrong the predictable way: `application/x-oxplow-task` ended up
 * declared twice — once in the (long-dead) `ThreadRail` component and once
 * in `agent-context-dnd.ts` so the decoder could avoid importing the React
 * tree — with a unit test whose entire job was to assert the two copies
 * hadn't drifted apart. One home means there's nothing to drift (tsk271).
 *
 * Kept free of imports on purpose, so a decoder, a pure helper, or a React
 * component can all reach a MIME without dragging anything else along.
 */

/** Task rows: reorder, restatus, and multi-select moves. Payload carries
 *  `itemIds` (plus a legacy single `itemId`) and an optional resolved
 *  `items` slice — see `agent-context-dnd.ts`. */
export const TASK_DRAG_MIME = "application/x-oxplow-task";

/** "Add to agent context" — a single typed reference (file, note, task)
 *  dropped onto the agent terminal. */
export const CONTEXT_REF_MIME = "application/x-oxplow-context-ref";

/** Rail HUD section reorder. */
export const RAIL_SECTION_DRAG_MIME = "application/x-oxplow-rail-section";

/** A work item card on the Board, carrying its ref (`work_item:oxplow:tsk4`). */
export const WORK_ITEM_DRAG_MIME = "application/x-oxplow-work-item";

/** Every MIME above, for invariant checks. Extend when adding one. */
export const ALL_DRAG_MIMES = [
  WORK_ITEM_DRAG_MIME,
  TASK_DRAG_MIME,
  CONTEXT_REF_MIME,
  RAIL_SECTION_DRAG_MIME,
] as const;
