import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useState } from "react";

import { SchemaForm } from "./SchemaForm.js";

afterEach(cleanup);

const schema = {
  type: "object",
  required: ["title"],
  properties: {
    title: { type: "string" },
    count: { type: ["integer", "null"] },
  },
};

test("submit waits for a valid value; Enter submits it", () => {
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} onSubmit={(v) => submitted.push(v)} submitLabel="Create" testIdPrefix="f" />,
  );
  const submit = getByTestId("f-submit") as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  fireEvent.change(getByTestId("f-title"), { target: { value: "Fix it" } });
  fireEvent.change(getByTestId("f-count"), { target: { value: "2" } });
  expect(submit.disabled).toBe(false);
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "Fix it", count: 2 }]);
});

test("a field's problem shows under it and keeps submit disabled", () => {
  const { getByTestId } = render(<SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="f" />);
  fireEvent.change(getByTestId("f-title"), { target: { value: "x" } });
  fireEvent.change(getByTestId("f-count"), { target: { value: "1.5" } });
  expect(getByTestId("f-count-error").textContent).toContain("whole number");
  expect((getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(true);
});

test("Escape returns to the initial value; onChange reports each edit", () => {
  const seen: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} initial={{ title: "Start" }} onChange={(v) => seen.push(v)} testIdPrefix="f" />,
  );
  const title = getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "" } });
  expect(seen.at(-1)).toBe(null);
  fireEvent.keyDown(title, { key: "Escape" });
  expect(title.value).toBe("Start");
  expect(seen.at(-1)).toEqual({ title: "Start" });
});

// Escape inside a form (or a command's confirmation) is that control's own
// cancel: it stops there, so a container's Escape — the Answers strip
// collapsing — doesn't also fire.
test("Escape in the form, or in a confirmation, stays inside it", async () => {
  const { CommandConfirm } = await import("../CommandConfirm.js");
  let outer = 0;
  const { getByTestId } = render(
    <div onKeyDown={(e) => e.key === "Escape" && outer++}>
      <SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="f" />
      <CommandConfirm label="Finish" command="oxplow.work_item.transition" onConfirm={() => {}} onCancel={() => {}} testIdPrefix="c" />
    </div>,
  );
  fireEvent.keyDown(getByTestId("f-title"), { key: "Escape" });
  fireEvent.keyDown(getByTestId("c-run"), { key: "Escape" });
  expect(outer).toBe(0);
});

// tsk1054: a host re-reading its data passes a fresh copy of the same
// schema and starting value; that's no reason to throw away an edit. A
// different starting value is.
test("an edit survives a fresh copy of the same starting value", () => {
  const view = render(<SchemaForm schema={schema} initial={{ title: "Start" }} testIdPrefix="f" />);
  const title = () => view.getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title(), { target: { value: "Edited" } });
  view.rerender(<SchemaForm schema={structuredClone(schema)} initial={{ title: "Start" }} testIdPrefix="f" />);
  expect(title().value).toBe("Edited");
  view.rerender(<SchemaForm schema={schema} initial={{ title: "Saved elsewhere" }} testIdPrefix="f" />);
  expect(title().value).toBe("Saved elsewhere");
});

const wide = {
  type: "object",
  required: ["title"],
  properties: {
    title: { type: "string" },
    note: { type: "string" },
    ref: { type: "string" },
  },
};

test("`fields` renders only those, in that order; the others are still submitted", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId, container } = render(
    <SchemaForm
      schema={wide}
      initial={{ title: "T", note: "hidden", ref: "r:1" }}
      fields={["note", "title", "nope"]}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  expect(queryByTestId("f-ref")).toBe(null);
  const ids = [...container.querySelectorAll("input")].map((i) => i.dataset.testid);
  expect(ids).toEqual(["f-note", "f-title"]);
  fireEvent.change(getByTestId("f-note"), { target: { value: "shown" } });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "T", note: "shown", ref: "r:1" }]);
});

test("a `fixed` field is not drawn, and is submitted unchanged", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId, container } = render(
    <SchemaForm schema={wide} initial={{ title: "T", ref: "r:1" }} fixed={["ref"]} onSubmit={(v) => submitted.push(v)} testIdPrefix="f" />,
  );
  expect(queryByTestId("f-ref-fixed")).toBe(null);
  expect(queryByTestId("f-ref")).toBe(null);
  expect(container.textContent).not.toContain("r:1");
  expect(document.activeElement).toBe(getByTestId("f-title"));
  expect(container.querySelectorAll("input").length).toBe(2);
  fireEvent.keyDown(getByTestId("f-title"), { key: "Escape" });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "T", ref: "r:1" }]);
});

test("a server error shows under its field until that field is edited; submit stays enabled", () => {
  const view = render(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.getByTestId("f-title-error").textContent).toBe("taken");
  expect((view.getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(false);
  // A re-read of the same errors keeps it cleared once edited.
  fireEvent.change(view.getByTestId("f-note"), { target: { value: "n" } });
  expect(view.queryByTestId("f-title-error")).not.toBe(null);
  fireEvent.change(view.getByTestId("f-title"), { target: { value: "T2" } });
  expect(view.queryByTestId("f-title-error")).toBe(null);
  view.rerender(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.queryByTestId("f-title-error")).toBe(null);
  view.rerender(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken again" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.getByTestId("f-title-error").textContent).toBe("taken again");
});

test("a server error for a field that isn't shown appears once above the submit button", () => {
  const { getByTestId, queryByTestId } = render(
    <SchemaForm
      schema={wide}
      initial={{ title: "T", ref: "r:1" }}
      fields={["title"]}
      serverErrors={{ ref: "no such ref", zzz: "unknown" }}
      onSubmit={() => {}}
      testIdPrefix="f"
    />,
  );
  expect(queryByTestId("f-ref-error")).toBe(null);
  expect(getByTestId("f-form-error").textContent).toContain("no such ref");
  expect(getByTestId("f-form-error").textContent).toContain("unknown");
  expect((getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(false);
});

const handoff = {
  type: "object",
  required: ["to_thread", "body"],
  properties: {
    to_thread: { type: "string" },
    body: { type: "string", contentMediaType: "text/markdown" },
  },
};

test("a markdown field is the Markdown editor: Enter adds a line, Cmd/Ctrl+Enter submits", async () => {
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={handoff}
      initial={{ to_thread: "oxplow:oxplow/thread?id=thr2" }}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  const body = await waitFor(() => getByTestId("f-body")) as HTMLElement & {
    editor?: { commands: { insertContent(text: string): boolean } };
  };
  expect(body.getAttribute("contenteditable")).toBe("true");
  await act(async () => {
    body.editor?.commands.insertContent("line one");
  });
  fireEvent.keyDown(body, { key: "Enter" });
  expect(submitted).toEqual([]);
  fireEvent.keyDown(body, { key: "Enter", metaKey: true });
  expect(submitted).toEqual([{ to_thread: "oxplow:oxplow/thread?id=thr2", body: "line one" }]);
});

test("a field with choices is a select of their labels, under their groups", () => {
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={handoff}
      initial={{ body: "yours" }}
      choices={{
        to_thread: [
          { value: "oxplow:oxplow/thread?id=thr2", label: "Parser", group: "oxplow" },
          { value: "oxplow:oxplow/thread?id=thr5", label: "Docs", group: "Bugfixes" },
        ],
      }}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  const select = getByTestId("f-to_thread") as HTMLSelectElement;
  expect(select.tagName).toBe("SELECT");
  const groups = Array.from(select.querySelectorAll("optgroup")).map((g) => g.label);
  expect(groups).toEqual(["oxplow", "Bugfixes"]);
  expect(Array.from(select.options).map((o) => o.textContent)).toEqual(["Choose…", "Parser", "Docs"]);
  fireEvent.change(select, { target: { value: "oxplow:oxplow/thread?id=thr5" } });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ to_thread: "oxplow:oxplow/thread?id=thr5", body: "yours" }]);
});

const provider = {
  type: "object",
  required: ["kind", "url", "token"],
  properties: {
    kind: { type: "string", enum: ["file", "http", "https"] },
    url: { type: "string" },
    token: { type: "string" },
    note: { type: ["string", "null"] },
  },
};
const PROVIDER_FIELDS = ["kind", { name: "url", visible_if: { kind: ["http", "https"] } }, { name: "token", secret: true, visible_if: { kind: "https" } }];

// A field shown only for some values of another: hidden, it is neither
// drawn, required nor submitted.
test("a field shows only while another holds one of its values", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId } = render(
    <SchemaForm schema={provider} fields={PROVIDER_FIELDS} onSubmit={(v) => submitted.push(v)} testIdPrefix="f" />,
  );
  fireEvent.change(getByTestId("f-kind"), { target: { value: "file" } });
  expect(queryByTestId("f-url")).toBeNull();
  expect(queryByTestId("f-token")).toBeNull();
  fireEvent.submit(getByTestId("f"));
  fireEvent.change(getByTestId("f-kind"), { target: { value: "http" } });
  fireEvent.change(getByTestId("f-url"), { target: { value: "http://x" } });
  expect(queryByTestId("f-token")).toBeNull();
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ kind: "file" }, { kind: "http", url: "http://x" }]);
});

// A secret is masked, never handed out as a draft, and blank keeps the
// saved one: it is left out, not required.
test("a secret field is masked, kept out of the draft, and left out when blank", () => {
  const submitted: unknown[] = [];
  const drafts: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={provider}
      fields={PROVIDER_FIELDS}
      initial={{ kind: "https", url: "https://x" }}
      onDraft={(d) => drafts.push(d)}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  const token = getByTestId("f-token") as HTMLInputElement;
  expect(token.type).toBe("password");
  fireEvent.submit(getByTestId("f"));
  fireEvent.change(token, { target: { value: "s3cret" } });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([
    { kind: "https", url: "https://x" },
    { kind: "https", url: "https://x", token: "s3cret" },
  ]);
  expect(JSON.stringify(drafts)).not.toContain("s3cret");
});

// Editing what is saved: Save only when changed, Discard puts it back.
test("an edit form saves only when changed, and Discard puts the saved value back", () => {
  const saved: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} mode="edit" initial={{ title: "Start" }} onSubmit={(v) => saved.push(v)} testIdPrefix="f" />,
  );
  const save = getByTestId("f-submit") as HTMLButtonElement;
  const discard = getByTestId("f-discard") as HTMLButtonElement;
  expect(save.textContent).toBe("Save");
  expect([save.disabled, discard.disabled]).toEqual([true, true]);
  const title = getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "Changed" } });
  expect([save.disabled, discard.disabled]).toEqual([false, false]);
  fireEvent.click(discard);
  expect(title.value).toBe("Start");
  expect(save.disabled).toBe(true);
  fireEvent.change(title, { target: { value: "Changed" } });
  fireEvent.submit(getByTestId("f"));
  expect(saved).toEqual([{ title: "Changed" }]);
});

// Its edits can be kept by the host (a tab's history state): null is
// untouched, so the form shows its starting value, whatever that is now.
test("a kept draft is shown; untouched follows the starting value", () => {
  const drafts: unknown[] = [];
  const view = render(
    <SchemaForm schema={schema} initial={{ title: "Start" }} draft={{ title: "Mine" }} onDraft={(d) => drafts.push(d)} testIdPrefix="f" />,
  );
  expect((view.getByTestId("f-title") as HTMLInputElement).value).toBe("Mine");
  view.rerender(<SchemaForm schema={schema} initial={{ title: "Theirs" }} draft={{ title: "Mine" }} onDraft={(d) => drafts.push(d)} testIdPrefix="f" />);
  expect((view.getByTestId("f-title") as HTMLInputElement).value).toBe("Mine");
  view.rerender(<SchemaForm schema={schema} initial={{ title: "Theirs" }} draft={null} onDraft={(d) => drafts.push(d)} testIdPrefix="f" />);
  expect((view.getByTestId("f-title") as HTMLInputElement).value).toBe("Theirs");
  cleanup();
  // A host keeping it: an edit back to the start is untouched again.
  function Host() {
    const [kept, setKept] = useState<Record<string, string> | null>(null);
    return (
      <SchemaForm
        schema={schema}
        initial={{ title: "Theirs" }}
        draft={kept}
        onDraft={(d) => {
          drafts.push(d);
          setKept(d);
        }}
        testIdPrefix="f"
      />
    );
  }
  const host = render(<Host />);
  const title = host.getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "New" } });
  expect(title.value).toBe("New");
  fireEvent.change(title, { target: { value: "Theirs" } });
  expect(drafts).toEqual([{ title: "New" }, null]);
});

test("a second submit sends the same value as its own, and Enter keeps the primary", () => {
  const sent: string[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={schema}
      onSubmit={(v) => sent.push(`save ${JSON.stringify(v)}`)}
      again={{ label: "Save and Another", onSubmit: (v) => sent.push(`again ${JSON.stringify(v)}`) }}
      submitLabel="Save"
      testIdPrefix="f"
    />,
  );
  const again = getByTestId("f-again") as HTMLButtonElement;
  expect(again.textContent).toBe("Save and Another");
  expect(again.disabled).toBe(true);
  fireEvent.change(getByTestId("f-title"), { target: { value: "One" } });
  expect(again.disabled).toBe(false);
  fireEvent.click(again);
  fireEvent.submit(getByTestId("f"));
  expect(sent).toEqual(['again {"title":"One"}', 'save {"title":"One"}']);
});

test("an optional field whose list of choices is empty isn't shown", () => {
  const { queryByTestId } = render(
    <SchemaForm
      schema={{ type: "object", properties: { title: { type: "string" }, parent: { type: ["string", "null"] } } }}
      choices={{ parent: [] }}
      testIdPrefix="f"
    />,
  );
  expect(queryByTestId("f-title")).not.toBeNull();
  expect(queryByTestId("f-parent")).toBeNull();
});

test("an object field with no children isn't shown", () => {
  const { queryByText } = render(
    <SchemaForm schema={{ type: "object", properties: { native: { type: "object", properties: {} } } }} testIdPrefix="f" />,
  );
  expect(queryByText("Native")).toBeNull();
});

test("a field the schema marks writeOnly is a secret: masked, never in the draft", () => {
  const drafts: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={{ type: "object", properties: { id: { type: "string" }, key: { type: "string", writeOnly: true } } }}
      initial={{ id: "or" }}
      mode="edit"
      onDraft={(d) => drafts.push(d)}
      onSubmit={() => {}}
      testIdPrefix="f"
    />,
  );
  const key = getByTestId("f-key") as HTMLInputElement;
  expect(key.type).toBe("password");
  fireEvent.change(key, { target: { value: "sk-secret" } });
  expect(JSON.stringify(drafts)).not.toContain("sk-secret");
});

// A command's schema is written for agents: a form for a person labels its
// fields by the host's `fields` (label, help), else the humanized name, and
// never shows the schema's description unless the host says it is for people.
const agentSchema = {
  type: "object",
  required: ["session", "title"],
  properties: {
    session: { type: "string", description: "The session (`ses3`)." },
    title: { type: "string" },
    native_rename: { type: "boolean", description: "Only where the harness declares `native_rename`." },
  },
};

test("fixed fields are not drawn, not even as text, but are still submitted", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId, container } = render(
    <SchemaForm schema={agentSchema} initial={{ session: "ses1", title: "T" }} fixed={["session"]} schemaHelp={false} onSubmit={(v) => submitted.push(v)} testIdPrefix="f" />,
  );
  expect(queryByTestId("f-session")).toBeNull();
  expect(queryByTestId("f-session-fixed")).toBeNull();
  expect(container.textContent).not.toContain("ses1");
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ session: "ses1", title: "T" }]);
});

test("a field's label and help come from the host's fields; the fallback is the humanized name, with no schema description", () => {
  const { getByText, queryByText, container } = render(
    <SchemaForm
      schema={agentSchema}
      fixed={["session"]}
      schemaHelp={false}
      fields={[{ name: "title", label: "Name", help: "What the tab shows." }, "native_rename"]}
      onSubmit={() => {}}
      testIdPrefix="f"
    />,
  );
  expect(getByText("Name")).toBeTruthy();
  expect(getByText("What the tab shows.")).toBeTruthy();
  expect(getByText("Native rename")).toBeTruthy();
  expect(container.textContent).not.toContain("harness declares");
  expect(queryByText("Title")).toBeNull();
});

test("a schema description shows only when the host says the schema is for people", () => {
  const { container } = render(<SchemaForm schema={agentSchema} onSubmit={() => {}} testIdPrefix="f" />);
  expect(container.textContent).toContain("harness declares");
});

test("a boolean is a checkbox with its label to the right and its help beneath", () => {
  const { getByTestId, getByText } = render(
    <SchemaForm
      schema={agentSchema}
      fixed={["session"]}
      initial={{ title: "T" }}
      schemaHelp={false}
      fields={["title", { name: "native_rename", label: "Also rename it in Codex", help: "Shown in Codex's list." }]}
      onSubmit={() => {}}
      testIdPrefix="f"
    />,
  );
  const box = getByTestId("f-native_rename") as HTMLInputElement;
  expect(box.type).toBe("checkbox");
  const label = getByText("Also rename it in Codex");
  expect(label.tagName).toBe("LABEL");
  expect(label.getAttribute("for")).toBe(box.id);
  expect(box.compareDocumentPosition(label) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(getByText("Shown in Codex's list.")).toBeTruthy();
});

test("a field picked from a list shows the labels, submits the value, and starts on the initial one", () => {
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm
      schema={{ type: "object", properties: { conversation: { type: "string" } } }}
      initial={{ conversation: "c2" }}
      choices={{ conversation: [{ value: "c1", label: "Fix the build" }, { value: "c2", label: "Add forms — this session's" }] }}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  const select = getByTestId("f-conversation") as HTMLSelectElement;
  expect(select.value).toBe("c2");
  expect(Array.from(select.options).map((o) => o.textContent)).toContain("Fix the build");
  fireEvent.change(select, { target: { value: "c1" } });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ conversation: "c1" }]);
});

test("Cancel, and Escape, cancel the form; Enter still submits", () => {
  let cancelled = 0;
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} initial={{ title: "T" }} onSubmit={(v) => submitted.push(v)} onCancel={() => cancelled++} submitLabel="Rename" testIdPrefix="f" />,
  );
  fireEvent.click(getByTestId("f-cancel"));
  expect(cancelled).toBe(1);
  fireEvent.keyDown(getByTestId("f-title"), { key: "Escape" });
  expect(cancelled).toBe(2);
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "T" }]);
});

test("the submit is a primary button, disabled only for a missing required field, and says why on hover", () => {
  const { getByTestId } = render(
    <SchemaForm schema={agentSchema} fixed={["session"]} initial={{ session: "s" }} schemaHelp={false} fields={[{ name: "title", label: "Name" }]} onSubmit={() => {}} submitLabel="Rename" testIdPrefix="f" />,
  );
  const submit = getByTestId("f-submit") as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  expect(submit.title).toBe("Fill in Name first");
  expect(submit.className).toContain("ox-primary");
  fireEvent.change(getByTestId("f-title"), { target: { value: "x" } });
  expect(submit.disabled).toBe(false);
  expect(submit.title).toBe("");
});

test("a form that opens takes the focus only when no other field has it", () => {
  const { getByTestId } = render(<SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="f" />);
  expect(document.activeElement).toBe(getByTestId("f-title"));
  cleanup();

  // A form that mounts on its own (the session picker of a new thread)
  // while the person types in the launcher must not take the keys.
  const typing = document.createElement("input");
  document.body.appendChild(typing);
  typing.focus();
  const later = render(<SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="g" />);
  expect(document.activeElement).toBe(typing);
  expect(later.getByTestId("g-title")).not.toBe(document.activeElement);
  typing.remove();
});
