import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { ICONS, iconFor } from "./icons.js";

// An icon is a name oxplow draws or the svg an extension brought (a data
// URI): the svg is the mask of an empty element, colored by the text
// around it like a built-in icon, its markup never injected.

afterEach(cleanup);

const svg = `data:image/svg+xml;base64,${btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>')}`;

test("a name oxplow draws is its icon", () => {
  expect(iconFor("plus")).toBe(ICONS.plus);
});

test("an extension's svg draws as a mask in the text's color, sized like a built-in", () => {
  const Icon = iconFor(svg)!;
  expect(Icon).not.toBeNull();
  expect(iconFor(svg)).toBe(Icon);
  const view = render(<Icon size={14} />);
  const el = view.getByTestId("svg-icon");
  expect(el.style.backgroundColor).toBe("currentcolor");
  expect(el.style.width).toBe("14px");
  expect(el.style.maskImage || el.style.webkitMaskImage).toContain(svg);
  expect(el.innerHTML).toBe("");
});

test("anything else draws nothing", () => {
  expect(iconFor("unicorn")).toBeNull();
  expect(iconFor(null)).toBeNull();
  expect(iconFor('data:image/svg+xml;utf8,<svg onload="x()"/>')).toBeNull();
});
