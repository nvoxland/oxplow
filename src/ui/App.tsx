import { useEffect, useState } from "react";
import { getCurrentStream, type Stream } from "./api.js";
import { TopBar } from "./components/TopBar.js";
import { LeftPanel } from "./components/LeftPanel.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { MainTabs } from "./components/MainTabs.js";

export function App() {
  const [stream, setStream] = useState<Stream | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getCurrentStream().then(setStream).catch((e) => setError(String(e)));
  }, []);

  return (
    <div
      style={{
        display: "grid",
        gridTemplateRows: "auto 1fr auto",
        gridTemplateColumns: "240px 1fr",
        height: "100vh",
        gap: 0,
      }}
    >
      <div style={{ gridColumn: "1 / 3", borderBottom: "1px solid var(--border)" }}>
        <TopBar stream={stream} error={error} />
      </div>
      <div style={{ borderRight: "1px solid var(--border)", overflow: "auto" }}>
        <LeftPanel />
      </div>
      <div style={{ overflow: "hidden", minHeight: 0 }}>
        {stream ? <MainTabs stream={stream} /> : <div style={{ padding: 12 }}>loading…</div>}
      </div>
      <div style={{ gridColumn: "1 / 3", borderTop: "1px solid var(--border)" }}>
        <BottomPanel />
      </div>
    </div>
  );
}
