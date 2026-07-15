export interface MenuItem {
  id: string;
  label: string;
  enabled: boolean;
  separator?: boolean;
  checked?: boolean;
  shortcut?: string;
  submenu?: MenuItem[];
  run?(): void | Promise<void>;
  /** This command only NAVIGATES to a page already in the pages
   *  directory (Files/Git/Tasks dashboards, etc.). It stays in the
   *  native menu bar, but the launcher's `flattenCommands` skips it so it
   *  doesn't duplicate the canonical `page` row. */
  opensPage?: boolean;
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
