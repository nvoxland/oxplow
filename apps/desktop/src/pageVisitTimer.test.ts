import { describe, expect, test } from "bun:test";
import { PageVisitTimer } from "./pageVisitTimer.js";

function harness(visible = true) {
  let t = 0;
  const sent: Array<[string, number]> = [];
  const timer = new PageVisitTimer((id, ms) => sent.push([id, ms]), () => t, visible);
  return { timer, sent, advance: (ms: number) => { t += ms; } };
}

describe("PageVisitTimer", () => {
  test("a visit is finished with how long it was shown when the next one begins", () => {
    const { timer, sent, advance } = harness();
    timer.begin().identify("pv1");
    advance(1500);
    timer.begin().identify("pv2");
    expect(sent).toEqual([["pv1", 1500]]);
    advance(200);
    timer.end();
    expect(sent).toEqual([["pv1", 1500], ["pv2", 200]]);
  });

  test("time with the window hidden is not counted", () => {
    const { timer, sent, advance } = harness();
    timer.begin().identify("pv1");
    advance(1000);
    timer.setVisible(false);
    advance(60_000);
    timer.setVisible(true);
    advance(500);
    timer.end();
    expect(sent).toEqual([["pv1", 1500]]);
  });

  test("a visit that began while hidden counts from when the window shows", () => {
    const { timer, sent, advance } = harness(false);
    timer.begin().identify("pv1");
    advance(5000);
    timer.setVisible(true);
    advance(300);
    timer.end();
    expect(sent).toEqual([["pv1", 300]]);
  });

  test("a visit that ends before its id is known is sent once the id arrives", () => {
    const { timer, sent, advance } = harness();
    const first = timer.begin();
    advance(800);
    timer.begin();
    expect(sent).toEqual([]);
    first.identify("pv1");
    expect(sent).toEqual([["pv1", 800]]);
  });

  test("ending twice sends once, and a visit never identified is never sent", () => {
    const { timer, sent, advance } = harness();
    timer.begin();
    advance(100);
    timer.end();
    timer.end();
    const v = timer.begin();
    v.identify("pv2");
    v.identify("pv3");
    advance(50);
    timer.end();
    timer.end();
    expect(sent).toEqual([["pv2", 50]]);
  });
});
