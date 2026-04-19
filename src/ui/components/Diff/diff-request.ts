export interface DiffRequest {
  path: string;
  leftRef: string;
  rightKind: "working" | { ref: string };
  baseLabel: string;
}
