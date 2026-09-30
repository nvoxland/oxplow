import { useCallback, useEffect, useState } from "react";
import { Page } from "../tabs/Page.js";
import { writeWikiPage } from "../api.js";
import { readWikiFreshness, readWikiPage } from "../knowledge.js";
import type { WikiRefFreshness } from "../knowledge.js";
import { fileRef, wikiPageRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { useOptionalPageNavigation, usePageTitle } from "../tabs/PageNavigationContext.js";

export interface WikiFreshnessPageProps {
  slug: string;
  onOpenPage(ref: TabRef): void;
}

/**
 * Per-wiki Freshness view. Lists every file ref the page carries with
 * the snapshot it was captured against, the latest snapshot of the
 * target file, and a "stale" flag. Two affordances: per-ref "Mark
 * verified" and page-level "Mark all verified" re-write the page with
 * those refs in `verified_refs` (knowledge.write_page), moving their
 * pins to the current snapshot.
 *
 * The wiki sync preserves unchanged ref pins on save, so this list
 * accurately reflects "the source files this page relies on have
 * changed since the page last verified them" — not "the page was
 * saved recently."
 */
export function WikiFreshnessPage({ slug, onOpenPage }: WikiFreshnessPageProps) {
  usePageTitle(`Freshness — ${slug}`);
  const nav = useOptionalPageNavigation();
  const [rows, setRows] = useState<WikiRefFreshness[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setRows(await readWikiFreshness(slug));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [slug]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Marking refs verified is re-writing the page with them in
  // `verified_refs` (knowledge.write_page moves their pins to now).
  async function markVerified(paths: string[]) {
    setBusy(true);
    try {
      const page = await readWikiPage(slug);
      if (!page) throw new Error(`no wiki page \`${slug}\``);
      await writeWikiPage(slug, page.body, { verifiedRefs: paths });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    await refresh();
    setBusy(false);
  }

  const markOne = (path: string) => markVerified([path]);
  const markAll = () => markVerified((rows ?? []).map((r) => r.path));

  const staleCount = rows?.filter((r) => r.stale).length ?? 0;

  return (
    <Page testId="page-wiki-freshness" kind="wiki-freshness" title={`Freshness — ${slug}`}>
      <div style={{ padding: "16px 24px", display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ display: "flex", gap: 12, alignItems: "baseline", flexWrap: "wrap" }}>
          <button
            type="button"
            onClick={() => {
              const ref = wikiPageRef(slug);
              if (nav) nav.navigate(ref);
              else onOpenPage(ref);
            }}
            style={{
              background: "transparent",
              border: "none",
              padding: 0,
              color: "var(--accent)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
              textDecoration: "underline",
            }}
          >
            ← Open wiki page
          </button>
          <span style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
            {rows == null
              ? "Loading…"
              : rows.length === 0
                ? "No file references"
                : `${staleCount} of ${rows.length} stale`}
          </span>
          {rows && rows.length > 0 ? (
            <button
              type="button"
              onClick={() => void markAll()}
              disabled={busy}
              style={{
                marginLeft: "auto",
                padding: "4px 10px",
                background: "var(--accent)",
                color: "var(--text-on-accent, white)",
                border: "none",
                borderRadius: 4,
                cursor: busy ? "wait" : "pointer",
                fontSize: "var(--text-xs)",
                opacity: busy ? 0.6 : 1,
              }}
            >
              Mark all verified
            </button>
          ) : null}
        </div>
        {error ? (
          <div style={{ color: "var(--severity-critical)", fontSize: "var(--text-sm)" }}>{error}</div>
        ) : null}
        {rows && rows.length > 0 ? (
          <table style={{
            width: "100%",
            borderCollapse: "collapse",
            fontSize: "var(--text-sm)",
          }}>
            <thead>
              <tr style={{ textAlign: "left", color: "var(--text-secondary)" }}>
                <th style={cellHeaderStyle}>File</th>
                <th style={cellHeaderStyle}>Captured snapshot</th>
                <th style={cellHeaderStyle}>Status</th>
                <th style={cellHeaderStyle} />
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.path} style={{ borderTop: "1px solid var(--border-subtle)" }}>
                  <td style={cellStyle}>
                    <button
                      type="button"
                      onClick={() => {
                        const ref = fileRef(row.path);
                        if (nav) nav.navigate(ref);
                        else onOpenPage(ref);
                      }}
                      style={{
                        background: "transparent",
                        border: "none",
                        padding: 0,
                        color: "var(--accent)",
                        cursor: "pointer",
                        textAlign: "left",
                        font: "inherit",
                        fontFamily: "var(--font-mono)",
                        textDecoration: "underline",
                      }}
                    >
                      {row.path}
                    </button>
                  </td>
                  <td style={{ ...cellStyle, color: "var(--text-secondary)" }}>
                    {row.pinned_snapshot_id === null ? "—" : `s-${row.pinned_snapshot_id}`}
                    {row.pinned_vcs_rev ? (
                      <span style={{ marginLeft: 6, color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>
                        {row.pinned_vcs_rev_exact ? "=" : "~"} {row.pinned_vcs_rev.slice(0, 8)}
                      </span>
                    ) : null}
                  </td>
                  <td style={cellStyle}>
                    {row.stale ? (
                      <span style={{
                        color: "var(--priority-high)",
                        fontSize: "var(--text-xs)",
                        fontWeight: 600,
                      }}>⚠ stale</span>
                    ) : (
                      <span style={{
                        color: "var(--diff-add-fg)",
                        fontSize: "var(--text-xs)",
                      }}>✓ fresh</span>
                    )}
                  </td>
                  <td style={cellStyle}>
                    {row.stale ? (
                      <button
                        type="button"
                        onClick={() => void markOne(row.path)}
                        disabled={busy}
                        style={{
                          padding: "2px 8px",
                          background: "transparent",
                          color: "var(--accent)",
                          border: "1px solid var(--border-subtle)",
                          borderRadius: 4,
                          cursor: busy ? "wait" : "pointer",
                          fontSize: "var(--text-xs)",
                          opacity: busy ? 0.6 : 1,
                        }}
                      >
                        Mark verified
                      </button>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : null}
      </div>
    </Page>
  );
}

const cellHeaderStyle: React.CSSProperties = {
  padding: "6px 10px",
  fontWeight: 500,
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: 0.4,
  borderBottom: "1px solid var(--border-subtle)",
};

const cellStyle: React.CSSProperties = {
  padding: "8px 10px",
  verticalAlign: "top",
};
