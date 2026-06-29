import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { EndpointPicker, type EndpointSnapshotOption } from "./EndpointPicker.js";

afterEach(cleanup);

const OPTIONS: EndpointSnapshotOption[] = [
  { snapshotId: 7, createdAt: "2026-06-29T05:24:24Z", gitCommit: "d1e4dcb0000" },
  { snapshotId: 5, createdAt: "2026-06-29T01:52:50Z", gitCommit: null },
];

test("trigger shows the short (time-only) label; the menu is closed initially", () => {
  const { getByTestId, queryByTestId } = render(
    <EndpointPicker
      testId="ep-start"
      ariaLabel="Start of range"
      triggerText="9:52:50 PM"
      currentSnapshotId={5}
      options={OPTIONS}
      onPick={() => {}}
    />,
  );
  expect(getByTestId("ep-start-trigger").textContent).toContain("9:52:50 PM");
  expect(queryByTestId("ep-start-option-5")).toBeNull();
});

test("opening lists the snapshots with full date+time and a commit sha", () => {
  const { getByTestId } = render(
    <EndpointPicker
      testId="ep-start"
      ariaLabel="Start of range"
      triggerText="9:52:50 PM"
      currentSnapshotId={5}
      options={OPTIONS}
      onPick={() => {}}
    />,
  );
  fireEvent.click(getByTestId("ep-start-trigger"));
  const withCommit = getByTestId("ep-start-option-7");
  // Full date+time in the dropdown (a year, not just a time) …
  expect(withCommit.textContent).toContain("2026");
  // … plus the short commit sha when the snapshot pinned a commit.
  expect(withCommit.textContent).toContain("d1e4dcb");
  // A snapshot with no commit shows no sha.
  expect(getByTestId("ep-start-option-5").textContent).not.toContain("d1e4dcb");
});

test("picking a snapshot fires onPick with its id and closes the menu", () => {
  const onPick = mock(() => {});
  const { getByTestId, queryByTestId } = render(
    <EndpointPicker
      testId="ep-end"
      ariaLabel="End of range"
      triggerText="12:24:24 AM"
      currentSnapshotId={null}
      options={OPTIONS}
      onPick={onPick}
    />,
  );
  fireEvent.click(getByTestId("ep-end-trigger"));
  fireEvent.click(getByTestId("ep-end-option-7"));
  expect(onPick).toHaveBeenCalledWith(7);
  expect(queryByTestId("ep-end-option-7")).toBeNull();
});

test("Escape closes the open menu", () => {
  const { getByTestId, queryByTestId } = render(
    <EndpointPicker
      testId="ep-start"
      ariaLabel="Start of range"
      triggerText="9:52:50 PM"
      currentSnapshotId={5}
      options={OPTIONS}
      onPick={() => {}}
    />,
  );
  fireEvent.click(getByTestId("ep-start-trigger"));
  expect(queryByTestId("ep-start-option-5")).not.toBeNull();
  fireEvent.keyDown(window, { key: "Escape" });
  expect(queryByTestId("ep-start-option-5")).toBeNull();
});

test("with no options the trigger is disabled (nothing to pick)", () => {
  const { getByTestId } = render(
    <EndpointPicker
      testId="ep-start"
      ariaLabel="Start of range"
      triggerText="working tree"
      currentSnapshotId={null}
      options={[]}
      onPick={() => {}}
    />,
  );
  expect((getByTestId("ep-start-trigger") as HTMLButtonElement).disabled).toBe(true);
});
