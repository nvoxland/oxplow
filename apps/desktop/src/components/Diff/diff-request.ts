import type { Revision } from "../../revision.js";

export interface DiffRequest {
  path: string;
  leftVersion: Revision;
  rightVersion: Revision;
  baseLabel: string;
}
