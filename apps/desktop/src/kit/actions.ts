/**
 * What a page, a section or a toolbar can do, as data. Core draws every
 * action (`ActionButtons`, the nav bar): a page says what it offers and
 * never how it looks.
 */
export interface PageAction {
  /** Unique among its neighbours; the button's test id is `action-<id>`. */
  id: string;
  label: string;
  run(): void | Promise<void>;
  /** The one main action of its place: drawn as the accent button. In a
   *  page's nav bar it stays a button, never in the ☰ menu. */
  primary?: boolean;
  /** In a page's nav bar, a button rather than an item of its ☰ menu
   *  (a lens action's `prominent`). Several may be. */
  prominent?: boolean;
  disabled?: boolean;
  /** Running now: shown as `label…`, and not pressable. */
  busy?: boolean;
  /** Hover text. */
  title?: string;
}
