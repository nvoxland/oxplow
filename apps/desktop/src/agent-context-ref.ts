/**
 * Discriminated union + formatter for "things the user can add to the
 * agent's context". Format outputs a one-line text snippet ending with
 * a trailing space so the user can keep typing around it before
 * pressing Enter.
 *
 * Files and wiki pages use Claude Code's `@<path>` mention convention
 * so the agent reads the file on the next prompt. tasks have no
 * file form and instead get a short bracketed reference; the agent can
 * resolve the title to a body via `oxplow__get_task` if it cares.
 */

export type ContextRef =
  | { kind: "file"; path: string }
  | { kind: "wiki"; slug: string }
  | { kind: "task"; itemId: string; title: string; status: string };

export function formatContextMention(ref: ContextRef): string {
  if (ref.kind === "file") {
    return `@${ref.path} `;
  }
  if (ref.kind === "wiki") {
    return `@.oxplow/wiki/${ref.slug}.md `;
  }
  // tasks: keep the title as plain text but strip newlines and
  // collapse internal whitespace so the inserted snippet stays on one
  // line. Don't escape quotes — the agent reads it as plain text.
  const cleanTitle = ref.title.replace(/\s+/g, " ").trim();
  return `[oxplow task ${ref.itemId}: "${cleanTitle}" (${ref.status})] `;
}
