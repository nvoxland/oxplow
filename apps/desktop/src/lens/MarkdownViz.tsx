/**
 * `viz: markdown`: the first row's text (`markdown: { text }`, else its
 * first column) in the one Markdown component. With an edit action
 * (`edit: true`) it is the editor, and that action is its Save: the forms
 * rule (usability.md "A form that edits what is saved saves only when
 * changed") — the draft is the tab's history state (`markdown:<lens id>`),
 * Save is enabled only when it differs from the saved text, Discard puts
 * the saved text back, Cmd/Ctrl+S saves, and a re-read never wipes a
 * draft while a clean editor follows the saved text. The text is laid over
 * the action's edited field (`editField`) as a form action's input;
 * Save keeps the page.
 */
import { useState } from "react";

import type { LensRun } from "../api.js";
import { useLensActionRunner } from "./lensActionRunner.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import type { PageAction } from "../tauri-bridge/generated/bindings.js";
import { useDraft } from "../tabs/useViewParam.js";
import { useTabEditing } from "../tabs/PageNavigationContext.js";
import { rowRecord } from "./lensActions.js";
import { usePageComments } from "./pageComments.js";
import { useFormSubmit } from "./useFormSubmit.js";

/** The text a markdown lens shows: its `text` column of the first row,
 *  else the first column. */
export function markdownText(run: LensRun): string {
  const { columns, rows } = run.result;
  const named = run.lens.markdown?.text ?? null;
  const at = named === null ? 0 : Math.max(0, columns.indexOf(named));
  const cell = rows[0]?.[at] ?? null;
  return cell === null ? "" : String(cell);
}

export function MarkdownViz({ run, streamId }: { run: LensRun; streamId: string | null }) {
  const edit = run.lens.actions.find((a) => a.edit) ?? null;
  const text = markdownText(run);
  if (!edit) return <Markdown value={text} testId="lens-markdown" />;
  return <MarkdownEditor run={run} streamId={streamId} action={edit} saved={text} />;
}

function MarkdownEditor({ run, streamId, action, saved }: { run: LensRun; streamId: string | null; action: PageAction; saved: string }) {
  const comments = usePageComments();
  // What the person typed, this step of the tab's; null: untouched, the
  // editor shows the saved text, whatever it is now.
  const [draft, setDraft, clearDraft] = useDraft<string | null>(`markdown:${run.lens.id}`, null);
  // Just saved: shown until the re-read brings it, so the editor doesn't
  // flash the text from before the save.
  const [justSaved, setJustSaved] = useState<{ text: string; over: string } | null>(null);
  const shownSaved = justSaved && justSaved.over === saved ? justSaved.text : saved;
  const field = action.editField ?? "";
  const row = run.result.rows[0] ? rowRecord(run.result.columns, run.result.rows[0]) : null;
  const runLensAction = useLensActionRunner();
  const form = useFormSubmit(
    action.label,
    (input, confirmed) => runLensAction(run.lens.id, action.id, run.params, row, streamId, confirmed, input),
    (outcome) => {
      void outcome;
      clearDraft();
    },
  );
  const note = (md: string) => setDraft(md === shownSaved ? null : md);
  const save = (md: string) => {
    if (md === shownSaved || form.busy) return;
    setJustSaved({ text: md, over: saved });
    void form.submit({ [field]: md }, false);
  };
  const dirty = draft !== null;
  // The tab's ● and the menu's Save.
  useTabEditing({ dirty, save: () => draft !== null && save(draft) });
  const error = form.serverErrors?.[field] ?? null;
  return (
    <div data-testid="lens-markdown" className="ox-markdown-editor">
      {form.pending ? (
        <CommandConfirm
          label={action.label}
          command={action.command}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
          testIdPrefix="lens-markdown-confirm"
        />
      ) : null}
      <Markdown
        value={draft ?? shownSaved}
        testId="lens-markdown-editor"
        placeholder="Write here…"
        comments={comments ?? undefined}
        onChange={note}
        onSaveShortcut={(md) => {
          note(md);
          save(md);
        }}
      />
      {error ? (
        <p data-testid="lens-markdown-error" className="ox-error">
          {error}
        </p>
      ) : null}
      <div className="ox-markdown-editor-actions">
        <button type="button" data-testid="lens-markdown-save" disabled={!dirty || form.busy} onClick={() => draft !== null && save(draft)}>
          {action.label}
        </button>
        <button type="button" data-testid="lens-markdown-discard" disabled={!dirty || form.busy} onClick={clearDraft}>
          Discard
        </button>
      </div>
    </div>
  );
}
