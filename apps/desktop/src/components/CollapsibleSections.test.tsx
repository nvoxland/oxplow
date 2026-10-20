import { afterEach, describe, expect, it } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState, type ReactNode } from "react";

import { Section } from "../kit/index.js";
import { CollapsibleSections, SectionCollapseControls } from "./CollapsibleSections.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";

/** A tab entry's state, kept outside the page so it outlives a mount, as
 *  the session does. */
interface Entry {
  state: Record<string, unknown>;
}
const freshEntry = (): Entry => ({ state: {} });

function Host({ entry, children }: { entry: Entry; children: ReactNode }) {
  const [, tick] = useState(0);
  const nav = {
    historyState: entry.state,
    setHistoryState: (patch: Record<string, unknown>) => {
      entry.state = { ...entry.state, ...patch };
      tick((n) => n + 1);
    },
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{children}</PageNavigationContext.Provider>;
}

// Mirrors the real adoption: the controls render OUTSIDE the section stack
// (on Metrics, in the details rail), reaching state through context.
function view(sections: Array<{ id: string; label: string }> = SECTIONS, entry: Entry = freshEntry()) {
  return render(
    <Host entry={entry}>
    <CollapsibleSections testIdPrefix="t">
      <aside>
        <SectionCollapseControls />
      </aside>
      <main>
        {sections.map((s) => (
          <Section key={s.id} id={s.id} title={s.label} collapsible>
            <div data-testid={`body-${s.id}`}>rows for {s.label}</div>
          </Section>
        ))}
      </main>
    </CollapsibleSections>
    </Host>,
  );
}

const SECTIONS = [
  { id: "testing", label: "Tests" },
  { id: "static-rust", label: "Rust" },
];

describe("CollapsibleSections", () => {
  // RTL's auto-cleanup doesn't run under bun:test — every component test file
  // here registers it by hand, else renders pile up in `document.body` and the
  // queries hit "found multiple elements".
  afterEach(cleanup);

  it("renders every section expanded by default", () => {
    const { getByTestId } = view();
    expect(getByTestId("body-testing")).toBeTruthy();
    expect(getByTestId("body-static-rust")).toBeTruthy();
  });

  it("collapses just the clicked section, leaving siblings open", () => {
    const { getByTestId, queryByTestId } = view();
    fireEvent.click(getByTestId("t-section-toggle-testing"));
    expect(queryByTestId("body-testing")).toBeNull();
    expect(queryByTestId("body-static-rust")).toBeTruthy();
    // The header itself survives — that's the only way back.
    expect(getByTestId("t-section-toggle-testing").getAttribute("aria-expanded")).toBe("false");
  });

  it("re-expands on a second click", () => {
    const { getByTestId, queryByTestId } = view();
    fireEvent.click(getByTestId("t-section-toggle-testing"));
    fireEvent.click(getByTestId("t-section-toggle-testing"));
    expect(queryByTestId("body-testing")).toBeTruthy();
  });

  it("collapse all hides every body; expand all brings them back", () => {
    const { getByTestId, queryByTestId } = view();
    fireEvent.click(getByTestId("t-collapse-all"));
    expect(queryByTestId("body-testing")).toBeNull();
    expect(queryByTestId("body-static-rust")).toBeNull();
    fireEvent.click(getByTestId("t-expand-all"));
    expect(queryByTestId("body-testing")).toBeTruthy();
    expect(queryByTestId("body-static-rust")).toBeTruthy();
  });

  it("disables the all-button that would do nothing", () => {
    const { getByTestId } = view();
    // Everything starts expanded → Expand all is a no-op.
    expect((getByTestId("t-expand-all") as HTMLButtonElement).disabled).toBe(true);
    expect((getByTestId("t-collapse-all") as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(getByTestId("t-collapse-all"));
    expect((getByTestId("t-expand-all") as HTMLButtonElement).disabled).toBe(false);
    expect((getByTestId("t-collapse-all") as HTMLButtonElement).disabled).toBe(true);
  });

  it("restores the collapsed set on remount, scoped to its own entry", () => {
    const entry = freshEntry();
    const first = view(SECTIONS, entry);
    fireEvent.click(first.getByTestId("t-section-toggle-testing"));
    first.unmount();

    const again = view(SECTIONS, entry);
    expect(again.queryByTestId("body-testing")).toBeNull();
    expect(again.queryByTestId("body-static-rust")).toBeTruthy();
    again.unmount();

    // Another entry reads its own state — not this one's.
    const other = view();
    expect(other.queryByTestId("body-testing")).toBeTruthy();
  });

  it("keeps a section collapsed while it is filtered out of the list", () => {
    // The search box on Metrics can drop a whole section; the
    // remembered collapse must survive its absence (the storage read is
    // deliberately not reconciled against the rendered ids).
    const entry = freshEntry();
    const first = view(SECTIONS, entry);
    fireEvent.click(first.getByTestId("t-section-toggle-static-rust"));
    first.unmount();

    // Re-render with that section filtered away entirely...
    const filtered = view([{ id: "testing", label: "Tests" }], entry);
    // ...and Collapse all reads as available, since the only rendered section
    // is expanded — the absent one must not make it look all-collapsed.
    expect((filtered.getByTestId("t-collapse-all") as HTMLButtonElement).disabled).toBe(false);
    filtered.unmount();

    // Bring it back: still collapsed.
    const restored = view(SECTIONS, entry);
    expect(restored.queryByTestId("body-static-rust")).toBeNull();
  });

  it("leaves a filtered-out section's state alone when expanding all", () => {
    const entry = freshEntry();
    const first = view(SECTIONS, entry);
    fireEvent.click(first.getByTestId("t-collapse-all"));
    first.unmount();

    // Expand all while `static-rust` is hidden must not silently expand it.
    const filtered = view([{ id: "testing", label: "Tests" }], entry);
    fireEvent.click(filtered.getByTestId("t-expand-all"));
    filtered.unmount();

    const restored = view(SECTIONS, entry);
    expect(restored.queryByTestId("body-testing")).toBeTruthy();
    expect(restored.queryByTestId("body-static-rust")).toBeNull();
  });

  it("drives sections from a sibling subtree, not an ancestor of them", () => {
    // The real page renders the controls in Page's details RAIL and the sections
    // in Page's body — two different subtrees under one provider. If the
    // controls only worked when wrapping the sections, that placement would
    // silently no-op.
    const { getByTestId, queryByTestId } = view();
    fireEvent.click(getByTestId("t-collapse-all"));
    expect(queryByTestId("body-testing")).toBeNull();
    expect(queryByTestId("body-static-rust")).toBeNull();
  });

  it("hides the controls entirely when there are no sections to act on", () => {
    // Loading / empty state: two permanently-disabled buttons would be noise.
    const { queryByTestId } = render(
      <CollapsibleSections testIdPrefix="t">
        <SectionCollapseControls />
        <div>Loading…</div>
      </CollapsibleSections>,
    );
    expect(queryByTestId("t-expand-all")).toBeNull();
    expect(queryByTestId("t-collapse-all")).toBeNull();
  });

  it("renders nothing for controls used outside a provider", () => {
    const { queryByTestId } = render(<SectionCollapseControls />);
    expect(queryByTestId("t-expand-all")).toBeNull();
  });

  it("renders sections read-only-open outside a provider", () => {
    const { getByTestId } = render(
      <Section id="solo" title="Solo" collapsible>
        <div data-testid="body-solo">rows</div>
      </Section>,
    );
    expect(getByTestId("body-solo")).toBeTruthy();
  });

  it("puts the toggle, a real button, inside the heading", () => {
    const { getByTestId } = view();
    const toggle = getByTestId("t-section-toggle-testing");
    expect(toggle.tagName).toBe("BUTTON");
    expect(toggle.closest("h2")).not.toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
  });

  it("starts a defaultCollapsed section closed, and remembers when it is opened", () => {
    const sections = (
      <CollapsibleSections testIdPrefix="t">
        <SectionCollapseControls />
        <Section id="a" title="A" collapsible defaultCollapsed>
          <div data-testid="body-a">a</div>
        </Section>
        <Section id="b" title="B" collapsible>
          <div data-testid="body-b">b</div>
        </Section>
      </CollapsibleSections>
    );
    const entry = freshEntry();
    const first = render(<Host entry={entry}>{sections}</Host>);
    expect(first.queryByTestId("body-a")).toBeNull();
    expect(first.getByTestId("body-b")).toBeTruthy();
    fireEvent.click(first.getByTestId("t-section-toggle-a"));
    expect(first.getByTestId("body-a")).toBeTruthy();
    first.unmount();
    // A later visit finds it as it was left; the other stays open.
    const again = render(<Host entry={entry}>{sections}</Host>);
    expect(again.getByTestId("body-a")).toBeTruthy();
    // Collapse all closes what's rendered, Expand all opens it again.
    fireEvent.click(again.getByTestId("t-collapse-all"));
    expect(again.queryByTestId("body-a")).toBeNull();
    expect(again.queryByTestId("body-b")).toBeNull();
    fireEvent.click(again.getByTestId("t-expand-all"));
    expect(again.getByTestId("body-a")).toBeTruthy();
    expect(again.getByTestId("body-b")).toBeTruthy();
  });

  it("a section that isn't collapsible has no toggle", () => {
    const { container } = render(
      <Section id="plain" title="Plain">
        <div>rows</div>
      </Section>,
    );
    expect(container.querySelector("button")).toBeNull();
  });
});
