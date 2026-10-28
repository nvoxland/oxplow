import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

const page = `title: Component Quoll
category: Data
loaders:
  quolls:
    sql: SELECT 'spotted' AS kind UNION ALL SELECT 'northern'
view:
  stack:
  - { id: before, markdown: Above the component. }
  - { id: echo, component: { id: echo, props: { mood: calm } } }
`;

// It draws the rows of each loader it is fed, and its own props.
const index = `<!doctype html>
<meta charset="utf-8">
<ul id="out"></ul>
<p id="props"></p>
<script src="/component-lib/oxplow-component.js"></script>
<script src="app.js"></script>
`;
const app = `oxplow.connect().then((component) => {
  const draw = (data) => {
    document.getElementById("out").innerHTML = data.quolls.rows.map((r) => "<li>" + r[0] + "</li>").join("");
    document.getElementById("props").textContent = "mood " + component.props.mood;
  };
  draw(component.data);
  component.onUpdate(({ data }) => draw(data));
});
`;

test("a component node is fed the page's loader data and its own props", async ({ page: win, daemon }) => {
  await installExtension(daemon, "e2e-component", {
    manifest: "custom_components:\n  - { id: echo, title: Echo }\n",
    files: { "pages/quoll.yaml": page, "components/echo/index.html": index, "components/echo/app.js": app },
  });
  await win.goto("/");
  await openFromLauncher(win, "Component Quoll");
  const host = win.getByTestId("page-host").filter({ visible: true });
  await expect(host.getByTestId("node-before")).toContainText("Above the component.");
  const frame = host.getByTestId("node-echo").frameLocator('[data-testid="custom-component-frame"]');
  await expect(frame.locator("li")).toHaveText(["spotted", "northern"]);
  await expect(frame.locator("#props")).toHaveText("mood calm");
});
