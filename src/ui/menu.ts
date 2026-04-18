export interface MenuItem {
  id: string;
  label: string;
  enabled: boolean;
  checked?: boolean;
  shortcut?: string;
  submenu?: MenuItem[];
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
