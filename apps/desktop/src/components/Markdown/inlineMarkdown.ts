/** The elements one line of markdown keeps (`Markdown inline`, a
 *  `markdown` column, a component's `renderMarkdown` inline): its marks
 *  and links. Anything else — a paragraph, a list, a heading, a quote, a
 *  code block, an image — unwraps into the line, or (with no text of its
 *  own) drops. */
export const INLINE_ELEMENTS: readonly string[] = ["a", "strong", "em", "del", "code", "br"];
