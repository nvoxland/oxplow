import { LensSlots } from "../../lens/LensSlots.js";
import { numericRowId } from "../../lens/lensModel.js";
import type { DiffSpec } from "./DiffPane.js";

/**
 * The `diff.file.header` slot: lenses about the file a diff shows, as a
 * strip under the diff's header (`.context/extensions.md` → "Slots"). A
 * lens gets the file's path and the two revisions the diff reads, in
 * their wire form (`working`, `snap:<id>`, `git:<sha>`).
 */
export function DiffFileHeaderSlot({ streamId, spec }: { streamId: string; spec: DiffSpec }) {
  const stream = numericRowId(streamId);
  return (
    <LensSlots
      slot="diff.file.header"
      params={
        stream === null
          ? null
          : { path: spec.path, left_revision: spec.leftVersion, right_revision: spec.rightVersion, stream_id: stream }
      }
      streamId={streamId}
      variant="strip"
    />
  );
}
