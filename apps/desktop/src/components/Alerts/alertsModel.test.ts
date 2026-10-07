import { expect, test } from "bun:test";

import { alertsSummary, alertKeys, newAlerts, type AlertItems } from "./alertsModel.js";

const none: AlertItems = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, badges: [], hints: [] };

// One bell for everything that needs the person.
test("the bell counts every item, red for a problem, accent for a decision", () => {
  expect(alertsSummary(none)).toEqual({ count: 0, tone: "none" });
  expect(alertsSummary({ ...none, proposals: [{ id: "proposal:1", title: "x" }] })).toEqual({ count: 1, tone: "accent" });
  expect(alertsSummary({ ...none, badges: [{ id: "x/b", title: "B", message: "2" }] })).toEqual({ count: 1, tone: "accent" });
  expect(
    alertsSummary({
      ...none,
      proposals: [{ id: "proposal:1", title: "x" }],
      opErrors: [{ id: "oe-1", label: "Push" }],
      undelivered: 3,
      failedReactions: 1,
    }),
  ).toEqual({ count: 1 + 1 + 3 + 1, tone: "danger" });
});

// A toast once per new item: a proposal or a failed operation as it
// arrives, delivery as it starts failing, a badge as it starts firing.
test("new alerts are the items not seen before, once each", () => {
  const before = alertKeys({ ...none, proposals: [{ id: "proposal:1", title: "a" }], undelivered: 2 });
  const now: AlertItems = {
    ...none,
    proposals: [
      { id: "proposal:1", title: "a" },
      { id: "proposal:2", title: "Your agent wants to run config.set" },
    ],
    opErrors: [{ id: "oe-1", label: "Push failed" }],
    undelivered: 5,
    badges: [{ id: "x/b", title: "Waiting on You", message: "2 items" }],
  };
  expect(newAlerts(before, now).map((a) => a.message)).toEqual([
    "Your agent wants to run config.set",
    "Push failed",
    "Waiting on You: 2 items",
  ]);
  expect(newAlerts(alertKeys(now), now)).toEqual([]);
  // Delivery: a toast when it starts failing, not on each new letter.
  expect(newAlerts(alertKeys(none), { ...none, undelivered: 1 }).map((a) => a.message)).toEqual([
    "1 event couldn't be delivered",
  ]);
});

// A hint raised to the person is a notice: it counts, toasts once, and
// isn't a problem.
test("a hint for the person counts and toasts once", () => {
  const hint = { id: 4, kind: "oxplow-bundled/landed-in-progress", message: "“t” is still in progress", threadTitle: "Main" };
  expect(alertsSummary({ ...none, hints: [hint] })).toEqual({ count: 1, tone: "accent" });
  expect(newAlerts(alertKeys(none), { ...none, hints: [hint] }).map((a) => a.message)).toEqual(["“t” is still in progress"]);
  expect(newAlerts(alertKeys({ ...none, hints: [hint] }), { ...none, hints: [hint] })).toEqual([]);
});
