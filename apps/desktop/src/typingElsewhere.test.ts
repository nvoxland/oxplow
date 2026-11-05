import { afterEach, describe, expect, it } from "vitest";
import { typingElsewhere } from "./typingElsewhere.js";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("typingElsewhere", () => {
  it("is true while a text field outside the region holds the focus", () => {
    document.body.innerHTML = '<div id="region"><button>in</button></div><input id="other" />';
    document.getElementById("other")!.focus();
    expect(typingElsewhere(document.getElementById("region")!)).toBe(true);
  });

  it("is false when the focus is inside the region, on the body, or on a non-field", () => {
    document.body.innerHTML = '<div id="region"><input id="in" /></div><button id="b">x</button>';
    const region = document.getElementById("region")!;
    document.getElementById("in")!.focus();
    expect(typingElsewhere(region)).toBe(false);
    document.getElementById("b")!.focus();
    expect(typingElsewhere(region)).toBe(false);
    (document.activeElement as HTMLElement).blur();
    expect(typingElsewhere(region)).toBe(false);
  });
});
