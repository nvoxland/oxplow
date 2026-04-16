import { randomBytes } from "node:crypto";

export function createId(prefix: string, byteLength = 6): string {
  return `${prefix}-${randomBytes(byteLength).toString("hex")}`;
}
