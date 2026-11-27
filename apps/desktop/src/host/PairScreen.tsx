import { useEffect, useState } from "react";

import { getDevice } from "../device.js";
import { shellStorage } from "../shellStorage.js";
import { hostOrigin, keepHostToken } from "../tauri-bridge/transport.js";
import { askToPair, collectPairing, type DeviceKind, type PairAsk } from "./hostApi.js";
import {
  cardStyle,
  errorStyle,
  primaryButtonStyle,
  formStyle,
  inputStyle,
  rootStyle,
  subtitleStyle,
  titleStyle,
} from "../launcher/screenStyles.js";

const DEFAULT_NAME: Record<DeviceKind, string> = { desktop: "Browser", tablet: "Tablet", phone: "Phone" };

/** The pause between two asks whether the owner typed the code. */
const COLLECT_PAUSE_MS = 500;

/** Before anything else on a page the host served, when this browser has no
 *  token for the host: this device asks to pair, with a name, and shows the
 *  code the host gives it. The owner types that code on the computer
 *  (Oxplow ▸ Devices…, or `oxplow host pair <code>`); only then is this
 *  device paired, and it keeps the id and token the host made and loads
 *  again. `ask`, `collect`, `pause` and `done` are a test's stand-ins. */
export function PairScreen({
  ask = askToPair,
  collect = collectPairing,
  pause = COLLECT_PAUSE_MS,
  done = () => window.location.reload(),
}: {
  ask?: typeof askToPair;
  collect?: typeof collectPairing;
  pause?: number;
  done?: () => void;
}) {
  const kind = getDevice().kind;
  const [name, setName] = useState(DEFAULT_NAME[kind]);
  const [asking, setAsking] = useState(false);
  const [asked, setAsked] = useState<PairAsk | null>(null);
  const [error, setError] = useState<string | null>(null);
  const host = hostOrigin() ?? "";

  const submit = async () => {
    if (asking || name.trim().length === 0) return;
    setAsking(true);
    setError(null);
    try {
      setAsked(await ask(host, name, kind));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setAsking(false);
    }
  };

  // Wait for the owner: each collect waits a while at the host; ask again
  // until paired or the ask ended.
  useEffect(() => {
    if (!asked) return;
    let live = true;
    void (async () => {
      try {
        for (;;) {
          const paired = await collect(host, asked.request);
          if (!live) return;
          if (paired) {
            keepHostToken(paired.token);
            // The host's id for this device, so the daemon knows it as the host does.
            shellStorage.setItem("oxplow.device", paired.device.id);
            done();
            return;
          }
          await new Promise((r) => setTimeout(r, pause));
          if (!live) return;
        }
      } catch (e) {
        if (!live) return;
        setAsked(null);
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      live = false;
    };
  }, [asked, collect, done, host, pause]);

  return (
    <div data-testid="pair-screen" style={rootStyle}>
      <div style={cardStyle}>
        <header style={{ marginBottom: 20 }}>
          <h1 style={titleStyle}>Pair this device</h1>
          <p style={subtitleStyle}>
            {asked
              ? "On the computer that runs Oxplow, open Oxplow ▸ Devices… and type this code (or run oxplow host pair with it):"
              : "Name this device, then ask: it shows a code to type on the computer that runs Oxplow."}
          </p>
        </header>
        {asked ? (
          <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 12 }}>
            <div
              data-testid="pair-shown-code"
              style={{ fontSize: "var(--text-xl)", fontWeight: 600, letterSpacing: "0.12em", fontFamily: "var(--font-mono)" }}
            >
              {asked.code}
            </div>
            <div style={subtitleStyle}>Waiting for it there… The code lasts ten minutes.</div>
          </div>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
            style={{ ...formStyle, flexDirection: "column", alignItems: "stretch" }}
          >
            <input
              data-testid="pair-name"
              aria-label="This device's name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="This device's name"
              autoFocus
              disabled={asking}
              style={inputStyle}
            />
            <button
              type="submit"
              data-testid="pair-submit"
              disabled={asking || name.trim().length === 0}
              style={primaryButtonStyle}
            >
              {asking ? "Asking…" : "Show a Code"}
            </button>
          </form>
        )}
        <p style={{ ...subtitleStyle, marginTop: 16 }} data-testid="pair-trust">
          Did the browser warn about this page? Make this device trust the computer once:{" "}
          <a href={`${host}/host/ca.pem`} download="oxplow-root.crt">
            download its certificate
          </a>{" "}
          and install it (Android: Settings ▸ Security ▸ More ▸ Install a certificate ▸ CA certificate;
          iPhone, iPad: install the downloaded profile, then turn it on in Settings ▸ General ▸ About ▸
          Certificate Trust Settings), then load this page again.
        </p>
        {error ? (
          <div data-testid="pair-error" role="alert" style={{ ...errorStyle, marginTop: 12 }}>
            {error}
          </div>
        ) : null}
      </div>
    </div>
  );
}
