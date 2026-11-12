import {
  Activity,
  Archive,
  AtSign,
  BarChart3,
  Bell,
  BookOpen,
  Box,
  Bug,
  Calendar,
  CheckSquare,
  CircleCheck,
  CircleDot,
  CircleStop,
  Columns3,
  Copy,
  Database,
  Download,
  ExternalLink,
  FileText,
  Flag,
  Folder,
  FolderTree,
  GitBranch,
  GitCommit,
  GitCompare,
  GitMerge,
  GitPullRequest,
  History,
  Inbox,
  LayoutDashboard,
  Link,
  ListTree,
  Maximize2,
  MessageSquare,
  Package,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Server,
  Settings,
  Shield,
  Sparkles,
  Star,
  Tag,
  Terminal,
  Ticket,
  Trash2,
  TriangleAlert,
  User,
  Zap,
  type LucideIcon,
} from "lucide-react";

import { createElement, type CSSProperties } from "react";

import { isIconName, type IconName } from "./iconNames.js";

/**
 * What draws each icon a resource, a page or an action may name
 * (`iconNames.ts`, the Rust `ICONS` allowlist, which a Rust test keeps in
 * step with these keys).
 */
export const ICONS: Readonly<Record<IconName, LucideIcon>> = {
  "activity": Activity,
  "archive": Archive,
  "at-sign": AtSign,
  "bar-chart-3": BarChart3,
  "bell": Bell,
  "book-open": BookOpen,
  "box": Box,
  "bug": Bug,
  "calendar": Calendar,
  "check-square": CheckSquare,
  "circle-check": CircleCheck,
  "circle-dot": CircleDot,
  "circle-stop": CircleStop,
  "columns-3": Columns3,
  "copy": Copy,
  "database": Database,
  "download": Download,
  "external-link": ExternalLink,
  "file-text": FileText,
  "flag": Flag,
  "folder": Folder,
  "folder-tree": FolderTree,
  "git-branch": GitBranch,
  "git-commit": GitCommit,
  "git-compare": GitCompare,
  "git-merge": GitMerge,
  "git-pull-request": GitPullRequest,
  "history": History,
  "inbox": Inbox,
  "layout-dashboard": LayoutDashboard,
  "link": Link,
  "list-tree": ListTree,
  "maximize-2": Maximize2,
  "message-square": MessageSquare,
  "package": Package,
  "pencil": Pencil,
  "play": Play,
  "plus": Plus,
  "refresh-cw": RefreshCw,
  "server": Server,
  "settings": Settings,
  "shield": Shield,
  "sparkles": Sparkles,
  "star": Star,
  "tag": Tag,
  "terminal": Terminal,
  "ticket": Ticket,
  "trash-2": Trash2,
  "triangle-alert": TriangleAlert,
  "user": User,
  "zap": Zap,
};

/** What an extension's svg icon looks like on the wire (`assets/<name>.svg`,
 *  published as a data URI): base64 only, so it holds no quote or
 *  parenthesis and can't leave the CSS `url()`. */
const SVG_DATA_URI = /^data:image\/svg\+xml;base64,[A-Za-z0-9+/=]+$/;

const svgIcons = new Map<string, LucideIcon>();

/** An extension's svg as an icon that sizes like the built-in ones: the
 *  mask of an empty element (an image load, where no script in it runs),
 *  colored by the text around it, so it follows the theme. */
function svgIcon(src: string): LucideIcon {
  let icon = svgIcons.get(src);
  if (!icon) {
    const image = `url("${src}")`;
    icon = (({ size = 16, className, style }: { size?: number | string; className?: string; style?: CSSProperties }) =>
      createElement("span", {
        "aria-hidden": true,
        "data-testid": "svg-icon",
        className,
        style: {
          display: "inline-block",
          flex: "none",
          width: size,
          height: size,
          backgroundColor: "currentColor",
          WebkitMaskImage: image,
          maskImage: image,
          WebkitMaskSize: "contain",
          maskSize: "contain",
          WebkitMaskRepeat: "no-repeat",
          maskRepeat: "no-repeat",
          WebkitMaskPosition: "center",
          maskPosition: "center",
          ...style,
        },
      })) as unknown as LucideIcon;
    svgIcons.set(src, icon);
  }
  return icon;
}

/** What draws an icon a resource, a page, an action or a choice names: one
 *  oxplow draws (`ICONS`), or the svg its extension brought (a data URI);
 *  anything else, nothing. */
export function iconFor(icon: string | null | undefined): LucideIcon | null {
  if (!icon) return null;
  if (isIconName(icon)) return ICONS[icon];
  return SVG_DATA_URI.test(icon) ? svgIcon(icon) : null;
}
