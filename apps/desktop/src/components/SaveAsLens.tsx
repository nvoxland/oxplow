import { useState } from "react";

import { keepLensSpec, type LensViz, type Stream } from "../api.js";
import { slugify } from "../lens/lensModel.js";
import { recordOpError } from "../opErrors.js";
import { lensRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { LensChart } from "../tauri-bridge/generated/bindings.js";

/** Inline "Save as Lens" strip: extension + title → a lens kept in this
 *  stream's worktree (`oxplow.lens.keep` with a spec), then opens it. Enter
 *  saves, Escape cancels. What a custom page's shown spec offers (Explore
 *  Data's query). */
export function SaveAsLens({
  query,
  viz,
  chart = null,
  stream,
  onOpenPage,
}: {
  query: string;
  viz: LensViz;
  /** A chart's columns, kept in the lens's `chart:`. */
  chart?: LensChart | null;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [open, setOpen] = useState(false);
  const [extension, setExtension] = useState("mine");
  const [title, setTitle] = useState("");

  async function save() {
    if (!title.trim() || !extension.trim()) return;
    try {
      const kept = await keepLensSpec(
        { title: title.trim(), description: "", query, viz, ...(chart ? { chart } : {}) },
        extension.trim(),
        slugify(title),
        stream?.id ?? null,
      );
      setOpen(false);
      setTitle("");
      onOpenPage(lensRef(kept.lens));
    } catch (e) {
      recordOpError({ label: "Save lens", message: e instanceof Error ? e.message : String(e) });
    }
  }

  if (!open) {
    return (
      <button type="button" data-testid="explore-save-open" onClick={() => setOpen(true)}>
        Save as Lens
      </button>
    );
  }
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") void save();
    if (e.key === "Escape") setOpen(false);
  };
  return (
    <span style={{ display: "flex", gap: 6, alignItems: "center" }}>
      <input
        data-testid="explore-save-extension"
        value={extension}
        onChange={(e) => setExtension(e.target.value)}
        onKeyDown={onKey}
        title="Extension (folder under .oxplow/extensions/)"
        style={{ width: 90 }}
      />
      <input
        data-testid="explore-save-title"
        autoFocus
        value={title}
        placeholder="Lens title"
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={onKey}
      />
      <button type="button" data-testid="explore-save" disabled={!title.trim()} onClick={() => void save()}>
        Save
      </button>
    </span>
  );
}
