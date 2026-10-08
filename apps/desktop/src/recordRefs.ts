/// How a command's input names a record: by its canonical ref,
/// `<kind>:<id>` (`.context/refs.md`) — `thread:thr3`, never `thr3`. The
/// bus refuses a bare id at the field.

/** A thread (`thr3` → `thread:thr3`). */
export const threadRef = (id: string): string => `thread:${id}`;
/** A stream (`str1` → `stream:str1`). */
export const streamRef = (id: string): string => `stream:${id}`;
/** An effort (`eff12` → `effort:eff12`). */
export const effortRef = (id: string): string => `effort:${id}`;
/** A wiki comment (`cmt12` → `comment:cmt12`). */
export const commentRef = (id: string): string => `comment:${id}`;
