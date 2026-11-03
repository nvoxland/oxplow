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
 * whichever component happened to introduce it. Declared beside their
 * components, they went wrong the predictable way: a task MIME ended up
 * declared twice — once in the (long-dead) `ThreadRail` component and once
 * in the (equally dead) drag decoder so it could avoid importing the React
 * tree — with a unit test whose entire job was to assert the two copies
 * hadn't drifted apart. One home means there's nothing to drift (tsk271).
 *
 * Kept free of imports on purpose, so a decoder, a pure helper, or a React
 * component can all reach a MIME without dragging anything else along.
 */

/** Rail HUD section reorder. */
export const RAIL_SECTION_DRAG_MIME = "application/x-oxplow-rail-section";

/** A page's rows, dragged to where a drag action (`drag: true`) runs: the
 *  payload is `{ token, keys }` — the page view's token (a drop is taken
 *  only by the view the rows came from) and the dragged rows' keys
 *  (`rowInteract.rowKey`). */
export const VIZ_ROW_DRAG_MIME = "application/x-oxplow-viz-rows";

/** Every MIME above, for invariant checks. Extend when adding one. */
export const ALL_DRAG_MIMES = [
  RAIL_SECTION_DRAG_MIME,
  VIZ_ROW_DRAG_MIME,
] as const;
