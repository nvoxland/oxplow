import { useEffect, useState } from "react";
import { getCurrentStream, listStreams, probeDaemon, renameCurrentStream, switchStream, type Stream } from "./api.js";
import { TopBar } from "./components/TopBar.js";
import { LeftPanel } from "./components/LeftPanel.js";
import { BottomPanel } from "./components/BottomPanel.js";
import { MainTabs } from "./components/MainTabs.js";

export function App() {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [stream, setStream] = useState<Stream | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);

  useEffect(() => {
    Promise.all([listStreams(), getCurrentStream()])
      .then(([allStreams, current]) => {
        setStreams(allStreams);
        setStream(current);
        setError(null);
        setDaemonUnavailable(false);
      })
      .catch((e) => {
        setError(String(e));
        setDaemonUnavailable(true);
      });
  }, []);

  useEffect(() => {
    let cancelled = false;

    async function check() {
      const alive = await probeDaemon();
      if (cancelled) return;
      setDaemonUnavailable(!alive);
    }

    check();
    const timer = window.setInterval(check, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  async function handleSwitch(id: string) {
    try {
      const next = await switchStream(id);
      setStream(next);
      setError(null);
      setDaemonUnavailable(false);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRename(title: string) {
    try {
      const updated = await renameCurrentStream(title);
      setStream(updated);
      setStreams((prev) =>
        prev
          .map((candidate) => (candidate.id === updated.id ? updated : candidate))
          .sort((a, b) => a.created_at.localeCompare(b.created_at)),
      );
      setError(null);
      setDaemonUnavailable(false);
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }

  function handleStreamCreated(next: Stream) {
    setStreams((prev) => {
      const others = prev.filter((stream) => stream.id !== next.id);
      return [...others, next].sort((a, b) => a.created_at.localeCompare(b.created_at));
    });
    setStream(next);
    setError(null);
    setDaemonUnavailable(false);
  }

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
        <TopBar
          stream={stream}
          streams={streams}
          error={error}
          onSwitch={handleSwitch}
          onRename={handleRename}
          onStreamCreated={handleStreamCreated}
        />
      </div>
      <div style={{ borderRight: "1px solid var(--border)", overflow: "auto" }}>
        <LeftPanel stream={stream} />
      </div>
      <div style={{ overflow: "hidden", minHeight: 0 }}>
        {stream ? <MainTabs key={stream.id} stream={stream} /> : <div style={{ padding: 12 }}>loading…</div>}
      </div>
      <div style={{ gridColumn: "1 / 3", borderTop: "1px solid var(--border)" }}>
        <BottomPanel streamId={stream?.id ?? null} />
      </div>
      {daemonUnavailable ? <DaemonDownDialog /> : null}
    </div>
  );
}

function DaemonDownDialog() {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0, 0, 0, 0.65)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
        padding: 24,
      }}
    >
      <div
        style={{
          width: "min(520px, 100%)",
          background: "var(--bg-2)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 20,
          boxShadow: "0 12px 40px rgba(0, 0, 0, 0.4)",
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>Backend daemon disconnected</div>
        <div style={{ color: "var(--muted)", lineHeight: 1.5, marginBottom: 16 }}>
          The backend daemon was killed or is no longer reachable. Stream switching, terminal panes, and hook
          updates will not keep working until the daemon is started again.
        </div>
        <button
          onClick={() => window.location.reload()}
          style={{
            background: "var(--accent)",
            color: "#fff",
            border: "none",
            padding: "8px 14px",
            borderRadius: 4,
            cursor: "pointer",
            fontFamily: "inherit",
          }}
        >
          Reload after restart
        </button>
      </div>
    </div>
  );
}
