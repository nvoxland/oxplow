/**
 * The component kit: what every page is drawn with, core or extension
 * (`.context/pages-and-tabs.md` "Page archetypes and the kit"). Each part
 * renders the kit stylesheet's classes (`oxplow-kit.css`), which custom
 * components link too, so they all look alike. A page composes these; a
 * new local section, table or filter list is a review finding.
 */
export type { PageAction } from "./actions.js";
export { Approvals, ProposalCard, ProposalList } from "./Approvals.js";
export { ActionButtons } from "./ActionButtons.js";
export { DataTable, type DataColumn, type DataGroup, type DataTableProps, type RowProps } from "./DataTable.js";
export { DocumentLayout, type DocSection } from "./DocumentLayout.js";
export { Facts, type Fact } from "./Facts.js";
export { humanize } from "./humanize.js";
export { ParamList, ParamRow, type ParamListProps, type ParamRowProps } from "./ParamControls.js";
export { Popover, type PopoverProps } from "./Popover.js";
export { RailGroup, type RailGroupProps } from "./RailGroup.js";
export { Loading } from "./Loading.js";
export { placeParts, type PartAt, type Placed } from "./placeParts.js";
export { useSearchFilter, type SearchFilterOptions } from "./SearchFilter.js";
export { Section, type SectionProps } from "./Section.js";
export { Stat } from "./Stat.js";
export { Tile, TileGrid, TileSlot, type TileSize } from "./Tiles.js";
export { Toolbar, type ToolbarFilter } from "./Toolbar.js";
export { useListNav } from "./useListNav.js";
// Lives beside the agent prompts it offers; part of the kit all the same.
export { EmptyState } from "../components/Prompts/EmptyState.js";
