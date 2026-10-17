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
  /** The one main action of its place: drawn as the accent button. */
  primary?: boolean;
  disabled?: boolean;
  /** Running now: shown as `label…`, and not pressable. */
  busy?: boolean;
  /** Hover text. */
  title?: string;
}
