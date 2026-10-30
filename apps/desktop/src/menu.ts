export interface MenuItem {
  id: string;
  label: string;
  enabled: boolean;
  separator?: boolean;
  checked?: boolean;
  shortcut?: string;
  /** A colour chip before the label (a CSS colour, e.g. a `var(--…)`
   *  token): the item picks that colour. */
  swatch?: string;
  /** An icon oxplow draws (`ICONS` in `resources.ts`) before the label. */
  icon?: string;
  /** What the item does, as a tooltip. */
  hint?: string;
  submenu?: MenuItem[];
  /** One line above the submenu's items saying what they do. */
  submenuHeader?: string;
  run?(): void | Promise<void>;
}

export interface MenuGroup {
  id: string;
  label: string;
  items: MenuItem[];
}

export interface MenuPosition {
  x: number;
  y: number;
}
