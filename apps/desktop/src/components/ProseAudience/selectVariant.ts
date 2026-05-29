import type { ProseAudience } from "../../tabs/proseAudience.js";

/** Mirrors the backend `ProseVariants` (crates/oxplow-domain/src/prose.rs)
 *  as exported via tauri-specta: developer is canonical, the other two
 *  are optional and fall back to developer. */
export interface ProseVariants {
  developer: string;
  executive?: string | null;
  terse?: string | null;
}

/** The body for `audience`, falling back to developer when the requested
 *  variant is absent or empty — the same rule as `ProseVariants::get`. */
export function selectVariantBody(variants: ProseVariants, audience: ProseAudience): string {
  const chosen = audience === "executive" ? variants.executive : audience === "terse" ? variants.terse : variants.developer;
  return chosen != null && chosen !== "" ? chosen : variants.developer;
}

/** Which audiences have a non-empty stored variant. Developer is always
 *  available; executive/terse only when present. Drives the muted
 *  styling on the selector for variants the agent hasn't authored yet. */
export function availableVariants(variants: ProseVariants): Record<ProseAudience, boolean> {
  return {
    developer: true,
    executive: variants.executive != null && variants.executive !== "",
    terse: variants.terse != null && variants.terse !== "",
  };
}
