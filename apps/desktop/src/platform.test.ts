import { expect, test } from "bun:test";

import { platformFrom, revealLabel } from "./platform.js";

// Which OS the window is on, from the navigator's platform or user agent:
// it names the file manager in Open In ▸.
test("the platform is read from the navigator", () => {
  expect(platformFrom("MacIntel")).toBe("mac");
  expect(platformFrom("Win32")).toBe("windows");
  expect(platformFrom("Linux x86_64")).toBe("linux");
  expect(platformFrom("")).toBe("linux");
});

test("the file manager has its OS's name", () => {
  expect(revealLabel("mac")).toBe("Reveal in Finder");
  expect(revealLabel("windows")).toBe("Show in Explorer");
  expect(revealLabel("linux")).toBe("Show in File Manager");
});
