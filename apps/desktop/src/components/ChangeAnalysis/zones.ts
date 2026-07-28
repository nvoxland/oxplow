// Path → architectural zone, driven by the PROJECT's rules.
//
// Zones are declared in `.oxplow/project.yaml` (`zones:`), not baked in
// here (tsk251): what makes a file "the store layer" follows from how a
// particular repo is laid out, and oxplow works on any repo. A project
// with no rules classifies everything as `other` and the zone surfaces
// stay quiet.
//
// The matcher below must agree with the Rust one in
// `crates/oxplow-code-deps/src/zones.rs` (globset). Both are pinned by
// the shared fixture at `fixtures/zone-globs.json` — add a case there
// when either changes.

import type { ZoneRuleConfig } from "../../tauri-bridge/generated/bindings.js";

/** A zone label. Free-form: the project names its own. */
export type Zone = string;

/** Files no rule matched. */
export const ZONE_OTHER = "other";
/** Import targets outside the repo (third-party package). */
export const ZONE_EXTERNAL = "external";

/** Palette handed out to labels that declare no `color`, in order of
 *  first appearance in the table. Wraps if a project declares more
 *  zones than colours. */
const ZONE_PALETTE = [
  "#4f46e5",
  "#ea580c",
  "#0891b2",
  "#16a34a",
  "#9333ea",
  "#dc2626",
  "#ca8a04",
  "#0ea5e9",
  "#c026d3",
  "#0d9488",
  "#b91c1c",
  "#2563eb",
];

/** Neutral fills for the two computed labels — deliberately grey so a
 *  real zone never reads as "unclassified" and vice versa. */
const ZONE_OTHER_COLOR = "#94a3b8";
const ZONE_EXTERNAL_COLOR = "#6b7280";

/** Translate the supported glob subset to a RegExp: `**` spans
 *  directories, `*` and `?` stop at `/`, everything else is literal.
 *  Mirrors globset with `literal_separator(true)`. */
export function globToRegExp(pattern: string): RegExp {
  let out = "";
  for (let i = 0; i < pattern.length; i++) {
    const c = pattern[i];
    if (c === "*") {
      if (pattern[i + 1] === "*") {
        // `**/` also matches zero directories, so `**/x` matches `x`.
        if (pattern[i + 2] === "/") {
          out += "(?:.*/)?";
          i += 2;
        } else {
          out += ".*";
          i += 1;
        }
      } else {
        out += "[^/]*";
      }
    } else if (c === "?") {
      out += "[^/]";
    } else if ("\\^$.|+()[]{}".includes(c)) {
      out += `\\${c}`;
    } else {
      out += c;
    }
  }
  return new RegExp(`^${out}$`);
}

/** A rule compiled once so a render pass doesn't rebuild its RegExps. */
interface CompiledRule {
  matchers: RegExp[];
  zone: Zone;
}

export interface CompiledZoneRules {
  rules: CompiledRule[];
  /** Label → colour, resolved from `color:` or the palette. */
  colors: Record<Zone, string>;
}

export function compileZoneRules(rules: readonly ZoneRuleConfig[]): CompiledZoneRules {
  const compiled: CompiledRule[] = [];
  const colors: Record<Zone, string> = {
    [ZONE_OTHER]: ZONE_OTHER_COLOR,
    [ZONE_EXTERNAL]: ZONE_EXTERNAL_COLOR,
  };
  let paletteIdx = 0;
  for (const rule of rules) {
    compiled.push({ matchers: rule.match.map(globToRegExp), zone: rule.zone });
    if (colors[rule.zone] === undefined) {
      colors[rule.zone] = rule.color ?? ZONE_PALETTE[paletteIdx++ % ZONE_PALETTE.length];
    }
  }
  return { rules: compiled, colors };
}

/** Classify a repo-relative path. First matching rule wins; `other`
 *  when none match (including a project with no rules at all). */
export function classifyZone(path: string, rules: CompiledZoneRules): Zone {
  const normalized = path.replace(/\\/g, "/");
  for (const rule of rules.rules) {
    if (rule.matchers.some((m) => m.test(normalized))) return rule.zone;
  }
  return ZONE_OTHER;
}

/** Display colour for a zone label. */
export function zoneColor(zone: Zone, rules: CompiledZoneRules): string {
  return rules.colors[zone] ?? ZONE_OTHER_COLOR;
}

/** The label as shown in chips/legends. Project labels are already
 *  short by convention, so this is identity today — kept as a seam so
 *  truncation/prettifying lands in one place. */
export function zoneLabel(zone: Zone): string {
  return zone;
}
