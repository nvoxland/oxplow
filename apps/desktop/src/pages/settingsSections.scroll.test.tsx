import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { Section } from "../kit/index.js";
import { scrollToSettingsSection } from "./settingsSections.js";

afterEach(cleanup);

// The settings index scrolls to a kit Section, which carries its id as
// `data-section`.
test("scrolling to a settings section finds the kit Section by its id", () => {
  const view = render(
    <div>
      <Section id="settings-agents" title="Agents" />
      <Section id="settings-ai" title="AI" />
    </div>,
  );
  const scrolled: string[] = [];
  for (const el of view.container.querySelectorAll("[data-section]")) {
    (el as HTMLElement).scrollIntoView = () => scrolled.push(el.getAttribute("data-section") ?? "");
  }
  scrollToSettingsSection("settings-ai");
  expect(scrolled).toEqual(["settings-ai"]);
});
