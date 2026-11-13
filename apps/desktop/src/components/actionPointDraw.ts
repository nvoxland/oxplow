/// How an action point is drawn (`.context/extensions-slots.md` "Action
/// points and the store filter"): nothing with no contributor, a button
/// with one, a menu with several. The same rule the daemon states
/// (`Draws::for_count`), kept here for the entries the window can run.
export type Draws = "none" | "button" | "menu";

export function drawsFor(contributors: number): Draws {
  if (contributors <= 0) return "none";
  return contributors === 1 ? "button" : "menu";
}
