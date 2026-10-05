import { expect, test } from "bun:test";

import { goToSettingsSection, SETTINGS_SECTIONS, takeSettingsSection, onSettingsSection } from "./settingsSections.js";

// tsk1040: an alert lands on its section of Settings — whether Settings is
// already open (it hears the request) or opens now (it takes the pending one).

test("a request waits for Settings to take it, once", () => {
  goToSettingsSection("settings-data-delivery");
  expect(takeSettingsSection()).toBe("settings-data-delivery");
  expect(takeSettingsSection()).toBeNull();
});

test("an open Settings hears each request, even the same one again", () => {
  const heard: string[] = [];
  const off = onSettingsSection((id) => heard.push(id));
  goToSettingsSection("settings-data-delivery");
  goToSettingsSection("settings-data-delivery");
  off();
  expect(heard).toEqual(["settings-data-delivery", "settings-data-delivery"]);
  // Heard, so not left pending for the next mount.
  expect(takeSettingsSection()).toBeNull();
});

test("the index names every section, Delivery among them", () => {
  expect(SETTINGS_SECTIONS.map((s) => s.id)).toContain("settings-data-delivery");
  expect(new Set(SETTINGS_SECTIONS.map((s) => s.id)).size).toBe(SETTINGS_SECTIONS.length);
});
