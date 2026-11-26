import { useState } from "react";

import { getDevice } from "../device.js";
import { shellStorage } from "../shellStorage.js";
import { hostOrigin, keepHostToken } from "../tauri-bridge/transport.js";
import { pairDevice, type DeviceKind } from "./hostApi.js";
import {
  cardStyle,
  errorStyle,
  remoteConnectButtonStyle,
  remoteFormStyle,
  remoteInputStyle,
  rootStyle,
  subtitleStyle,
  titleStyle,
} from "../launcher/screenStyles.js";

const DEFAULT_NAME: Record<DeviceKind, string> = { desktop: "Browser", tablet: "Tablet", phone: "Phone" };

/** Before anything else on a page the host served, when this browser has no
 *  token for the host: type the code the host's owner shows (Oxplow ▸
 *  Devices… on the Mac, or `oxplow host pair`) and a name for this device.
 *  The host makes the device's id and token; the page keeps both and loads
 *  again. `pair` and `done` are a test's stand-ins. */
export function PairScreen({
  pair = pairDevice,
  done = () => window.location.reload(),
}: {
  pair?: typeof pairDevice;
  done?: () => void;
}) {
  const kind = getDevice().kind;
  const [code, setCode] = useState("");
  const [name, setName] = useState(DEFAULT_NAME[kind]);
  const [pairing, setPairing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const host = hostOrigin() ?? "";

  const submit = async () => {
    if (pairing || code.trim().length === 0) return;
    setPairing(true);
    setError(null);
    try {
      const paired = await pair(host, code, name, kind);
      keepHostToken(paired.token);
      // The host's id for this device, so the daemon knows it as the host does.
      shellStorage.setItem("oxplow.device", paired.device.id);
      done();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPairing(false);
    }
  };

  return (
    <div data-testid="pair-screen" style={rootStyle}>
      <div style={cardStyle}>
        <header style={{ marginBottom: 20 }}>
          <h1 style={titleStyle}>Pair this device</h1>
          <p style={subtitleStyle}>
            On the computer that runs Oxplow, choose Oxplow ▸ Devices… ▸ Pair a Device (or run{" "}
            <code>oxplow host pair</code>), then type the code it shows.
          </p>
        </header>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
          style={{ ...remoteFormStyle, flexDirection: "column", alignItems: "stretch" }}
        >
          <input
            data-testid="pair-code"
            aria-label="Pairing code"
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase())}
            placeholder="ABCD-EFGH"
            autoFocus
            autoComplete="one-time-code"
            autoCapitalize="characters"
            spellCheck={false}
            disabled={pairing}
            style={remoteInputStyle}
          />
          <input
            data-testid="pair-name"
            aria-label="This device's name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="This device's name"
            disabled={pairing}
            style={remoteInputStyle}
          />
          <button
            type="submit"
            data-testid="pair-submit"
            disabled={pairing || code.trim().length === 0}
            style={remoteConnectButtonStyle}
          >
            {pairing ? "Pairing…" : "Pair"}
          </button>
        </form>
        {error ? (
          <div data-testid="pair-error" role="alert" style={{ ...errorStyle, marginTop: 12 }}>
            {error}
          </div>
        ) : null}
      </div>
    </div>
  );
}
