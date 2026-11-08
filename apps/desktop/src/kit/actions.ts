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
   *  (a page action's `prominent`). Several may be. */
  prominent?: boolean;
  /** One of oxplow's own actions every page of its kind has (a page
   *  page's Refresh, Copy, Add to Context): the
   *  ☰ keeps these together at the top, in the order given, and sorts the
   *  page's other actions by label. */
  standard?: boolean;
  /** The ☰ submenu it is in (a page action's `submenu`), shared with the
   *  commands whose group has that name. */
  submenu?: string;
  /** An icon oxplow draws (`ICONS` in `resources.ts`): alone on its
   *  button, the label the hover; beside its label in a menu. */
  icon?: string;
  disabled?: boolean;
  /** Running now: shown as `label…`, and not pressable. */
  busy?: boolean;
  /** Hover text. */
  title?: string;
  /** The key its failures show by (`useActionAnchor`): set it when `run`
   *  goes through the person's runner with its own anchor, so the failure
   *  shows beside this button. */
  anchor?: string;
}
