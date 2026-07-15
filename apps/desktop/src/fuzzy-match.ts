// Subsequence match — each character in the query must appear in order in
// the haystack, with any gap allowed. Matches Linear's feel ("wn" → "work /
// new tasks") without pulling in a scoring library. Used across the launcher
// (Cmd+P) — pages, commands, and file paths — so they all accept the same
// kind of terse typing.
export function fuzzyMatches(haystack: string, needle: string): boolean {
  if (!needle) return true;
  let ni = 0;
  for (let hi = 0; hi < haystack.length && ni < needle.length; hi++) {
    if (haystack[hi] === needle[ni]) ni++;
  }
  return ni === needle.length;
}
