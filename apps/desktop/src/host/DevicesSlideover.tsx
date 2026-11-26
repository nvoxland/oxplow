import { useCallback, useEffect, useState } from "react";

import {
  hostDevices,
  hostPairingCode,
  hostRemoveDevice,
  hostRenameDevice,
  hostTailscale,
  setHostSettings,
} from "../api.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import { InlineEdit } from "../components/InlineEdit.js";
import { Slideover } from "../components/Slideover.js";
import type { DevicesView, HostSettings, PairingCodeView } from "../tauri-bridge/generated/bindings.js";

/** The app menu's "Devices…" (`commands/menu.rs` `DEVICES_MENU_ID`). */
export const DEVICES_MENU_ID = "app.devices";

/** The calls this window makes; a test's stand-ins. */
export type DevicesApi = {
  hostDevices: typeof hostDevices;
  hostPairingCode: typeof hostPairingCode;
  hostRenameDevice: typeof hostRenameDevice;
  hostRemoveDevice: typeof hostRemoveDevice;
  setHostSettings: typeof setHostSettings;
  hostTailscale: typeof hostTailscale;
};

const DEVICES_API: DevicesApi = {
  hostDevices,
  hostPairingCode,
  hostRenameDevice,
  hostRemoveDevice,
  setHostSettings,
  hostTailscale,
};

/** The form's fields, as typed: names one per comma, paths as text. */
type Draft = { allow: boolean; address: string; names: string; cert: string; key: string };

function draftOf(s: HostSettings): Draft {
  return {
    allow: s.allowOtherDevices ?? false,
    address: s.address ?? "",
    names: (s.names ?? []).join(", "),
    cert: s.tlsCert ?? "",
    key: s.tlsKey ?? "",
  };
}

function settingsOf(d: Draft): HostSettings {
  const path = (p: string) => (p.trim() === "" ? null : p.trim());
  return {
    allowOtherDevices: d.allow,
    address: d.address.trim(),
    names: d.names
      .split(",")
      .map((n) => n.trim())
      .filter((n) => n.length > 0),
    tlsCert: path(d.cert),
    tlsKey: path(d.key),
  };
}

const same = (a: Draft, b: Draft) => JSON.stringify(a) === JSON.stringify(b);

/** `mm:ss` of a countdown. Pure; exported for tests. */
export function countdown(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Oxplow ▸ Devices… on the Mac: pair a tablet or another computer with this
 *  Mac's host, the paired devices (rename, remove), and "Allow other
 *  devices" (where they reach this Mac; it applies at the next start).
 *  Window-level, not about a tab, so a `Slideover`. Quitting Oxplow stops the
 *  host, and every device's projects with it. */
export function DevicesSlideover({
  open,
  onClose,
  api = DEVICES_API,
}: {
  open: boolean;
  onClose(): void;
  api?: DevicesApi;
}) {
  const [view, setView] = useState<DevicesView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [code, setCode] = useState<(PairingCodeView & { until: number }) | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [saved, setSaved] = useState<Draft | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);

  const read = useCallback(async () => {
    try {
      const next = await api.hostDevices();
      setView(next);
      const kept = draftOf(next.settings);
      setSaved(kept);
      setDraft((d) => d ?? kept);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [api]);

  useEffect(() => {
    if (open) void read();
  }, [open, read]);

  // The code's countdown, and the list again while it is good (a device
  // that pairs shows up).
  useEffect(() => {
    if (!open || code === null) return;
    const timer = setInterval(() => {
      setNow(Date.now());
      void read();
    }, 1_000);
    return () => clearInterval(timer);
  }, [open, code, read]);

  const left = code ? (code.until - now) / 1000 : 0;
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
      await read();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const changed = draft !== null && saved !== null && !same(draft, saved);
  const set = (patch: Partial<Draft>) => setDraft((d) => (d ? { ...d, ...patch } : d));

  return (
    <Slideover open={open} onClose={onClose} title="Devices" testId="devices">
      {error ? (
        <div role="alert" data-testid="devices-error" style={alertStyle}>
          {error}
        </div>
      ) : null}
      {view?.hostError ? (
        <div role="alert" data-testid="devices-host-error" style={alertStyle}>
          Oxplow's host did not answer: {view.hostError}. Fix the settings below and start Oxplow again.
        </div>
      ) : null}
      {view?.public?.state === "failed" ? (
        <div role="alert" data-testid="devices-public-failed" style={alertStyle}>
          Other devices cannot reach this Mac: {view.public.error}
        </div>
      ) : null}

      <section style={sectionStyle}>
        <h3 style={headStyle}>Pair a device</h3>
        <p style={mutedStyle}>
          A tablet, a phone or another computer uses Oxplow on this Mac through its browser. Quitting
          Oxplow here stops it for them too.
        </p>
        {code && left > 0 ? (
          <div data-testid="devices-code">
            <div style={codeStyle}>{code.code}</div>
            <div style={mutedStyle}>
              Good once, for {countdown(left)}. On the device, open{" "}
              {view?.openOnDevice.length ? (
                view.openOnDevice.map((url, i) => (
                  <span key={url}>
                    {i > 0 ? " or " : ""}
                    <code>{url}</code>
                  </span>
                ))
              ) : (
                <span>this Mac's address</span>
              )}{" "}
              and type the code.
            </div>
          </div>
        ) : (
          <button
            type="button"
            data-testid="devices-pair"
            onClick={() =>
              void run(async () => {
                const next = await api.hostPairingCode();
                setNow(Date.now());
                setCode({ ...next, until: Date.now() + next.expiresInSeconds * 1000 });
              })
            }
          >
            Pair a Device
          </button>
        )}
        {view && !view.settings.allowOtherDevices ? (
          <p style={mutedStyle} data-testid="devices-off">
            Other devices cannot reach this Mac yet: turn on Allow other devices below.
          </p>
        ) : null}
      </section>

      <section style={sectionStyle}>
        <h3 style={headStyle}>Paired devices</h3>
        {view === null ? (
          <div style={mutedStyle}>Loading…</div>
        ) : view.devices.length === 0 ? (
          <div style={mutedStyle} data-testid="devices-none">
            None yet.
          </div>
        ) : (
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {view.devices.map((d) => (
              <li key={d.id} data-testid={`device-${d.id}`} style={rowStyle}>
                <span
                  title={d.connected ? "Using Oxplow now" : "Not connected"}
                  style={{ ...dotStyle, background: d.connected ? "var(--accent)" : "var(--border-subtle)" }}
                />
                <span style={{ flex: 1 }}>
                  <InlineEdit
                    value={d.name}
                    testId={`device-name-${d.id}`}
                    ariaLabel="Device name"
                    onCommit={(name) => void run(() => api.hostRenameDevice(d.id, name))}
                  />
                  <span style={mutedStyle}>
                    {" "}
                    {d.kind} · paired {new Date(d.pairedAt * 1000).toLocaleDateString()}
                  </span>
                </span>
                <InlineConfirm
                  triggerLabel="Remove"
                  confirmLabel="Remove"
                  testIdPrefix={`device-remove-${d.id}`}
                  title="It must pair again to use this Mac; its open projects stop unless another device holds them"
                  onConfirm={() => void run(() => api.hostRemoveDevice(d.id))}
                />
              </li>
            ))}
          </ul>
        )}
      </section>

      {draft ? (
        <form
          style={sectionStyle}
          data-testid="devices-settings"
          onSubmit={(e) => {
            e.preventDefault();
            if (!changed) return;
            void run(async () => {
              await api.setHostSettings(settingsOf(draft));
              setSaved(draft);
            });
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape" && changed) {
              e.preventDefault();
              e.stopPropagation();
              setDraft(saved);
            }
          }}
        >
          <h3 style={headStyle}>Allow other devices</h3>
          <label style={fieldStyle}>
            <input
              type="checkbox"
              data-testid="devices-allow"
              checked={draft.allow}
              onChange={(e) => set({ allow: e.target.checked })}
            />{" "}
            Other devices may reach this Mac
          </label>
          <label style={fieldStyle}>
            Address
            <span style={{ display: "flex", gap: 6 }}>
              <input
                data-testid="devices-address"
                value={draft.address}
                placeholder="100.101.102.103 (this Mac on your private network)"
                onChange={(e) => set({ address: e.target.value })}
                style={{ flex: 1 }}
              />
              <button
                type="button"
                data-testid="devices-tailscale"
                onClick={() =>
                  void run(async () => {
                    const me = await api.hostTailscale();
                    if (me === null) throw new Error("Tailscale is not running on this Mac");
                    set({ address: me.address, names: me.name ?? draft.names });
                  })
                }
              >
                Use Tailscale
              </button>
            </span>
          </label>
          <label style={fieldStyle}>
            Names a device uses (comma between)
            <input
              data-testid="devices-names"
              value={draft.names}
              placeholder="mac.tailnet.ts.net"
              onChange={(e) => set({ names: e.target.value })}
            />
          </label>
          <label style={fieldStyle}>
            HTTPS certificate (PEM; <code>tailscale cert &lt;name&gt;</code> makes one)
            <input data-testid="devices-cert" value={draft.cert} onChange={(e) => set({ cert: e.target.value })} />
          </label>
          <label style={fieldStyle}>
            Its key (PEM)
            <input data-testid="devices-key" value={draft.key} onChange={(e) => set({ key: e.target.value })} />
          </label>
          <p style={mutedStyle}>
            {view?.settingsApplied
              ? "Oxplow uses these now."
              : "These apply the next time Oxplow starts (quit and open it again)."}{" "}
            Without HTTPS a device's browser has no microphone or clipboard here.
          </p>
          <div style={{ display: "flex", gap: 8 }}>
            <button type="submit" data-testid="devices-save" disabled={!changed}>
              Save
            </button>
            <button type="button" data-testid="devices-discard" disabled={!changed} onClick={() => setDraft(saved)}>
              Discard
            </button>
          </div>
        </form>
      ) : null}
    </Slideover>
  );
}

const sectionStyle: React.CSSProperties = { marginBottom: 20 };
const alertStyle: React.CSSProperties = { color: "var(--severity-critical)", marginBottom: 12 };
const headStyle: React.CSSProperties = {
  margin: "0 0 8px",
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: "var(--text-muted)",
};
const mutedStyle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const codeStyle: React.CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 28,
  letterSpacing: 3,
  margin: "4px 0",
};
const rowStyle: React.CSSProperties = { display: "flex", alignItems: "center", gap: 8, padding: "6px 0" };
const dotStyle: React.CSSProperties = { width: 8, height: 8, borderRadius: 4, flex: "none" };
const fieldStyle: React.CSSProperties = { display: "flex", flexDirection: "column", gap: 4, marginBottom: 10 };
