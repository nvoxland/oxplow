/// Tooltip / accessible label for a wiki index row (WikiPane NoteRow).
/// The count comes from the backend's `file_refs` list — interpolating
/// a field the DTO doesn't carry is how rows used to read
/// "undefined referenced files" to screen readers; when the field is
/// missing entirely, drop the clause instead.
export function wikiRowTooltip(note: {
  title: string;
  slug: string;
  file_refs?: string[] | null;
}): string {
  const count = Array.isArray(note.file_refs) ? note.file_refs.length : null;
  const slugLine =
    count != null
      ? `${note.slug} — ${count} referenced file${count === 1 ? "" : "s"}`
      : note.slug;
  return [note.title, slugLine, "Drag onto agent to add to context"].join("\n");
}
