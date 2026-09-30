/// Ids as the models hold them (integers) and as the UI names them
/// (`thr3`, `tsk42`): the one place the two meet.

/** A thread id (`thr12`) as its row id in the models. */
export function threadRowId(threadId: string): number {
  return Number(threadId.replace(/^thr/, ""));
}

/** A model's thread row id as the UI's thread id. */
export function threadIdOf(row: number): string {
  return `thr${row}`;
}

/** A model's task row id as the UI's task id. */
export function taskIdOf(row: number): string {
  return `tsk${row}`;
}

/** A stream id (`str2`) as its row id in the models. */
export function streamRowId(streamId: string): number {
  return Number(streamId.replace(/^str/, ""));
}

