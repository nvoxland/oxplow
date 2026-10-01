/**
 * An agent's runs that wait for a person (P6b, `.context/commands.md` →
 * "Proposals"): read from `v_command_proposal`, decided through RPC
 * `decide_proposal`. Approving runs the command as the person — the
 * approval is the confirmation; declining runs nothing.
 */
import { useCallback, useEffect, useState } from "react";

import { decideProposal, querySql } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { valueText } from "./pages/settingsModel.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** What the run raised: the command, its summary and input, and whether
 *  it is destructive. */
export interface ProposalPreview {
  command: string;
  summary: string;
  input: unknown;
  destructive: boolean;
}

/** One pending proposal. */
export interface Proposal {
  id: number;
  /** `proposal:<id>`. */
  ref: string;
  createdAt: string;
  command: string;
  input: unknown;
  /** `agent`, or `lens` for a lens acting for an agent. */
  actorKind: string;
  /** The proposing thread (`thr3`) or lens. */
  actorId: string | null;
  threadId: number | null;
  /** `config:<key>` for a config change, else the command and its input. */
  key: string;
  preview: ProposalPreview;
  /** What it would have done when proposed; `null` when it couldn't be
   *  dry-run (an External command). */
  dryRun: unknown;
}

/** How a card shows a proposal. */
export interface ProposalSummary {
  title: string;
  who: string;
  /** A config key's value before and after. */
  change: { before: string; after: string } | null;
  /** A composite's commands, in order. */
  children: string[];
  destructive: boolean;
}

const json = (text: SqlCell): unknown => {
  if (text == null) return null;
  try {
    return JSON.parse(String(text));
  } catch {
    return null;
  }
};

export function proposalsFromResult(result: SqlQueryResult): Proposal[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  return result.rows.map((row) => ({
    id: Number(at(row, "id")),
    ref: String(at(row, "ref")),
    createdAt: String(at(row, "created_at") ?? ""),
    command: String(at(row, "command")),
    input: json(at(row, "input")),
    actorKind: String(at(row, "actor_kind")),
    actorId: at(row, "actor_id") == null ? null : String(at(row, "actor_id")),
    threadId: at(row, "thread_id") == null ? null : Number(at(row, "thread_id")),
    key: String(at(row, "key")),
    preview: json(at(row, "preview")) as ProposalPreview,
    dryRun: json(at(row, "dry_run")),
  }));
}

/** The pending proposals, newest first, and what was read. */
export async function readPendingProposals(): Promise<{ proposals: Proposal[]; reads: Reads }> {
  const res = await querySql(
    `SELECT id, ref, created_at, command, input, actor_kind, actor_id, thread_id, key, preview, dry_run
       FROM v_command_proposal WHERE decision = 'pending' ORDER BY id DESC`,
    [],
    1_000,
  );
  return { proposals: proposalsFromResult(res), reads: res.reads };
}

/** The pending proposals, re-read when the model changes. */
export function useProposals(): Proposal[] {
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readPendingProposals()
      .then((r) => {
        setProposals(r.proposals);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        // The rail and Settings keep what they showed; the next change
        // retries. The person learns the list may be stale.
        recordOpError({ label: "Read pending proposals", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return proposals;
}

const field = (value: unknown, name: string): unknown =>
  value && typeof value === "object" ? (value as Record<string, unknown>)[name] : undefined;

export function summarizeProposal(p: Proposal): ProposalSummary {
  const key = field(p.input, "key");
  const title =
    p.command === "config.set" && typeof key === "string"
      ? `Set ${key}`
      : p.command === "config.unset" && typeof key === "string"
        ? `Unset ${key}`
        : p.preview?.summary || p.command;
  const agent = p.actorId ? `The agent in ${p.actorId}` : "The agent";
  const who = p.actorKind === "lens" ? `A lens${p.actorId ? ` (${p.actorId})` : ""} for the agent` : agent;
  const hasChange = p.dryRun != null && typeof p.dryRun === "object" && "before" in p.dryRun && "after" in p.dryRun;
  const shown = (v: unknown) => (v == null ? "(not set)" : valueText(v));
  const children = field(p.dryRun, "children");
  return {
    title,
    who,
    change: hasChange ? { before: shown(field(p.dryRun, "before")), after: shown(field(p.dryRun, "after")) } : null,
    children: Array.isArray(children) ? children.map((c) => String(field(c, "name"))) : [],
    destructive: Boolean(p.preview?.destructive),
  };
}

/** The pending proposal that changes setting `key`, if any. */
export function proposalForSetting(proposals: Proposal[], key: string): Proposal | undefined {
  return proposals.find((p) => p.key === `config:${key}`);
}

/** Approve or decline `p` as the person. */
export async function decide(p: Proposal, approve: boolean): Promise<void> {
  await decideProposal(p.id, approve);
}
