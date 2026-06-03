/**
 * Effort snapshot-id normalization.
 *
 * The Tauri binding types an effort's `start_snapshot_id` /
 * `end_snapshot_id` as `number | null`, but the app-facing `TaskEffort`
 * (and Rust's `TreeVersion.id`, which these flow into when opening an
 * effort-changes diff) use strings. Passing a raw number reaches Rust
 * as `{ kind: "snapshot", id: <number> }`; serde rejects the numeric
 * `id` against the `String` field and the invoke fails as an opaque
 * "ipc error". Normalize at the `api.ts` boundary so every consumer
 * gets the string the app type already promises.
 */
export function normalizeSnapshotId(id: number | string | null | undefined): string | null {
  return id == null ? null : String(id);
}
