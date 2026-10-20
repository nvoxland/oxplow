/// A declared page whose body is a custom component (`pages:` with
/// `component:`), full-page: core's chrome and nav bar (title and actions
/// drawn by core, never by the frame) around the component's sandboxed
/// frame. The frame says the page's title and actions over the bridge
/// (`setTitle`, `setActions`), asks for a declared command's form
/// (`openForm`, shown here as a lens's form action is) and is told the
/// page's params, stream and thread. Marked "custom".
import { useEffect, useMemo, useRef, useState } from "react";

import { componentForm, invokeComponentCommand } from "../api.js";
import { EmptyState } from "../components/Prompts/EmptyState.js";
import { recordOpError } from "../components/opErrorsStore.js";
import type { PageAction } from "../kit/actions.js";
import { ComponentFrame, CustomBadge, noteStyle, useComponentBundle, type BridgeHost } from "../lens/ComponentFrame.js";
import type { FrameAction, PageInit } from "../lens/componentBridge.js";
import { FormSlideover, useFormSubmit } from "../lens/LensResultView.js";
import { Page } from "../tabs/Page.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";
import { extPageRef, pageTabRef, type PagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { remoteBaseUrl } from "../tauri-bridge/transport.js";
import type { ExtensionPage, FormStart, Stream } from "../tauri-bridge/generated/bindings.js";

interface AskedForm {
  command: string;
  input: unknown;
  version: string;
  resolve(out: { result: unknown } | null): void;
  reject(e: unknown): void;
}

export function CustomPageView({
  page,
  declared,
  stream,
  threadId = null,
  base = remoteBaseUrl(),
  readyTimeoutMs = 3000,
}: {
  page: PagePayload;
  declared: ExtensionPage;
  stream: Stream | null;
  /** The thread it is shown in, which the frame is told. */
  threadId?: string | null;
  onOpenPage(ref: TabRef): void;
  /** The daemon that serves bundles; none means no component. */
  base?: string | null;
  readyTimeoutMs?: number;
}) {
  const componentKey = declared.body.kind === "component" ? declared.body.component : null;
  const bundle = useComponentBundle(componentKey, base);
  const [frameTitle, setFrameTitle] = useState<string | null>(null);
  const [frameActions, setFrameActions] = useState<FrameAction[]>([]);
  const [asked, setAsked] = useState<AskedForm | null>(null);
  const hostRef = useRef<BridgeHost | null>(null);
  // Why the frame at `src` was given up on; a new `src` starts afresh.
  const [failed, setFailed] = useState<{ src: string; reason: string } | null>(null);

  usePageTitle(frameTitle ?? declared.title);

  const actions: PageAction[] = useMemo(
    () =>
      frameActions.map((a) => ({
        id: a.id,
        label: a.label,
        primary: a.primary,
        disabled: a.disabled,
        run: () => hostRef.current?.action(a.id),
      })),
    [frameActions],
  );

  const streamId = stream?.id ?? null;
  const init: PageInit = useMemo(
    () => ({ params: page.params, streamId, threadId }),
    // `page.params` is a fresh object per render; its content is the key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [JSON.stringify(page.params), streamId, threadId],
  );
  // Its view is its tab entry's params: a change replaces the entry (a
  // component has no selection to push; going somewhere is `navigate`).
  const nav = useOptionalPageNavigation();
  const navRef = useRef(nav);
  navRef.current = nav;
  const pageRef = useRef(page);
  pageRef.current = page;
  const pageDeps = useMemo(
    () => ({
      setTitle: setFrameTitle,
      setActions: setFrameActions,
      setParams: (changes: Record<string, string | null>) => {
        const at = pageRef.current;
        const next: Record<string, string> = { ...at.params };
        for (const [k, v] of Object.entries(changes)) {
          if (v === null) delete next[k];
          else next[k] = v;
        }
        const ref = at.extension === null ? pageTabRef(at.page, next, at.state) : extPageRef(at.extension, at.page, next);
        navRef.current?.replaceRef?.(ref);
      },
      openForm: (command: string, input: unknown, version: string) =>
        new Promise<{ result: unknown } | null>((resolve, reject) => {
          setAsked({ command, input, version, resolve, reject });
        }),
    }),
    [],
  );

  let body;
  if (bundle.loadKey === null) {
    body = (
      <div style={{ padding: 16 }}>
        <EmptyState title="Needs the daemon" text="This page is an extension's own component, which the daemon serves. Connect to the daemon to see it." />
      </div>
    );
  } else if (bundle.error) {
    body = (
      <div data-testid="custom-component-fallback" style={{ ...noteStyle, padding: 16 }}>
        {`Its component couldn't be loaded: ${bundle.error}.`}
      </div>
    );
  } else if (bundle.src !== null && failed?.src === bundle.src) {
    body = (
      <div data-testid="custom-component-fallback" style={{ ...noteStyle, padding: 16 }}>
        {failed.reason}
      </div>
    );
  } else if (bundle.src === null || bundle.version === null || componentKey === null) {
    body = (
      <div data-testid="custom-component-loading" style={{ ...noteStyle, padding: 16 }}>
        Loading its component…
      </div>
    );
  } else {
    const src = bundle.src;
    body = (
      <div data-testid="custom-component" style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0, height: "100%" }}>
        <CustomBadge extension={declared.extension} />
        {/* Keyed by its URL: another version mounts a new frame. */}
        <ComponentFrame
          key={src}
          src={src}
          version={bundle.version}
          component={componentKey}
          run={null}
          page={init}
          pageDeps={pageDeps}
          hostRef={hostRef}
          title={frameTitle ?? declared.title}
          streamId={streamId}
          readyTimeoutMs={readyTimeoutMs}
          onFail={(why) => setFailed({ src, reason: why })}
        />
      </div>
    );
  }

  return (
    <Page testId="page-custom" kind="page" width="full" actions={actions}>
      {body}
      {asked && componentKey ? (
        <PageForm
          componentKey={componentKey}
          title={frameTitle ?? declared.title}
          asked={asked}
          onDone={() => setAsked(null)}
        />
      ) : null}
    </Page>
  );
}

/** A declared command's form the frame asked for, in a slideover: its
 *  fields start from the frame's input; submitting runs it as the
 *  component (the confirm, when it asks, is the host's) and answers the
 *  frame with its result; closing it answers none. */
function PageForm({
  componentKey,
  title,
  asked,
  onDone,
}: {
  componentKey: string;
  title: string;
  asked: AskedForm;
  onDone(): void;
}) {
  const [start, setStart] = useState<FormStart | null>(null);
  const answered = useRef(false);
  const answer = (out: { result: unknown } | null) => {
    if (answered.current) return;
    answered.current = true;
    asked.resolve(out);
    onDone();
  };
  const form = useFormSubmit(
    title,
    (input, confirmed) => invokeComponentCommand(componentKey, asked.command, input, confirmed, asked.version),
    (outcome) => answer({ result: outcome.result }),
  );
  useEffect(() => {
    let live = true;
    componentForm(componentKey, asked.command, asked.input, asked.version).then(
      (s) => {
        if (live) setStart(s);
      },
      (e: unknown) => {
        if (!live) return;
        recordOpError({ label: title, message: e instanceof Error ? e.message : String(e) });
        answered.current = true;
        asked.reject(e);
        onDone();
      },
    );
    return () => {
      live = false;
    };
    // The form asked for is `asked`; it is fixed for this mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [asked]);
  return (
    <FormSlideover
      title={title}
      start={start}
      form={form}
      submitLabel="Submit"
      onClose={() => answer(null)}
      testId="custom-page-form"
    />
  );
}
