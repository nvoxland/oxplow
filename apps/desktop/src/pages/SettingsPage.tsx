import { useEffect } from "react";
import { DocumentLayout, type DocSection } from "../kit/DocumentLayout.js";
import { useSearchFilter } from "../kit/SearchFilter.js";
import { Toolbar } from "../kit/Toolbar.js";
import type { PageAction } from "../kit/actions.js";
import { settingsRef, viewParamsOf } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { SETTINGS_AREAS, settingsArea, settingsSlot, type SettingsArea, type SettingsAreaId } from "./settingsAreas.js";

export interface SettingsPageProps {
  /** Which area (`page:settings#area=…`); the overview when absent. */
  area?: string | null;
  /** Where to land: one of the area's section ids, or an anchor inside one. */
  section?: string | null;
  /** Opens another area (or a link), in this tab. */
  onOpenPage(ref: TabRef): void;
}

/**
 * Settings, one Document page per area (`settingsAreas.ts`), all sharing a
 * rail that lists the areas. The overview is Settings as a view (P6.H1):
 * every setting, its value and where it comes from, with Ask the Agent to
 * Change This. The other areas hold the direct controls for what only a
 * person may set. Each area's sections are a slot an extension adds to
 * (`settings.<area>.sections`).
 */
export function SettingsPage({ area: areaId, section, onOpenPage }: SettingsPageProps) {
  const area = settingsArea(areaId);
  // Land on the section (or anchor) the link asked for.
  useEffect(() => {
    if (!section) return;
    const frame = requestAnimationFrame(() => {
      const target = document.querySelector(`[data-section="${CSS.escape(section)}"]`) ?? document.getElementById(section);
      target?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
    return () => cancelAnimationFrame(frame);
  }, [area.id, section]);
  // Another area is this page at another view: a step Back returns from,
  // not a page opened over it. Outside a tab, a link like any other.
  const nav = useOptionalPageNavigation();
  const open = (id: SettingsAreaId, anchor: string | null = null) => {
    const ref = settingsRef(id, anchor);
    if (nav?.setViewParams) nav.setViewParams(viewParamsOf(ref), { push: true });
    else onOpenPage(ref);
  };
  const { sections, actions } = useAreaContent(area);
  // The overview's search is the page's `q` view param; its lenses read it.
  const filter = useSearchFilter("q", {
    label: "Search settings",
    placeholder: "Search settings",
    testId: "settings-search",
    inputProps: {
      onKeyDown: (e) => {
        if (e.key === "Escape") filter.onChange("");
      },
    },
  });
  return (
    <DocumentLayout
      testId="page-settings"
      title={area.id === "overview" ? "Settings" : area.title}
      intro={area.intro}
      sections={sections}
      actions={actions}
      slot={settingsSlot(area.id)}
      slotParams={area.slotParams?.includes("q") ? { q: filter.value } : undefined}
      toolbar={area.slotParams?.includes("q") ? <Toolbar filter={filter} /> : undefined}
      onOpenPage={onOpenPage}
      rail={<SettingsAreasNav current={area.id} onOpen={(id) => open(id)} />}
      railTitle="Settings"
    />
  );
}

/** The rail every Settings page shares: the areas, the one shown marked. */
function SettingsAreasNav({ current, onOpen }: { current: SettingsAreaId; onOpen(id: SettingsAreaId): void }) {
  return (
    <nav aria-label="Settings areas" data-testid="settings-areas" style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
      {SETTINGS_AREAS.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`settings-area-${a.id}`}
          aria-current={a.id === current ? "page" : undefined}
          className="ox-link"
          onClick={() => onOpen(a.id)}
          style={{
            background: "none",
            border: "none",
            padding: 0,
            font: "inherit",
            textAlign: "left",
            fontWeight: a.id === current ? "var(--weight-bold)" : undefined,
            color: a.id === current ? "var(--text-primary)" : undefined,
          }}
        >
          {a.title}
        </button>
      ))}
    </nav>
  );
}

/** An area's own sections (titled as the area model titles them) and its
 *  page actions. */
function useAreaContent(area: SettingsArea): { sections: DocSection[]; actions?: PageAction[] } {
  switch (area.id) {
    case "overview":
    case "agents":
    case "lsp":
    case "ai":
    case "extensions":
    case "data":
    case "integrations":
      // Their sections are foundation lenses mounted in the area's slot.
      return { sections: [] };
  }
}

