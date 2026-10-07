import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { subscribeAgentInput } from "../agent-input-bus.js";
import { PageNavBar } from "./PageNavBar.js";

afterEach(cleanup);

// Ask About This (P6.D1): the open page's ref goes into the agent's
// input for the person to finish the question — never sent.
test("Ask About This puts the page's ref in the agent's input", async () => {
  const inserted: string[] = [];
  const off = subscribeAgentInput((t) => inserted.push(t));
  const view = render(
    <PageNavBar canBack={false} canForward={false} onBack={() => {}} onForward={() => {}} ask={{ ref: "commit:abc123" }} />,
  );
  fireEvent.click(view.getByTestId("page-nav-ask"));
  fireEvent.click(view.getByTestId("page-nav-ask-this"));
  expect(inserted).toEqual(["[oxplow ref commit:abc123] "]);
  off();
  cleanup();
  const none = render(<PageNavBar canBack={false} canForward={false} onBack={() => {}} onForward={() => {}} />);
  expect(none.queryByTestId("page-nav-ask")).toBeNull();
});
