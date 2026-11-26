import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  /** Optional label shown in the fallback header (e.g. tab name). */
  label?: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
  info: ErrorInfo | null;
}

/**
 * Generic React error boundary. Without it, a render error inside any
 * page component unmounts the entire React tree and the user sees a
 * black window with no clue what happened. The boundary catches the
 * error, logs it to the console, and renders a fallback panel showing
 * the message + stack so the user can copy-paste it back.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    this.setState({ info });
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary]", this.props.label ?? "", error, info?.componentStack);
  }

  reset = () => {
    this.setState({ error: null, info: null });
  };

  render(): ReactNode {
    const { error, info } = this.state;
    if (!error) return this.props.children;
    return (
      <div
        style={{
          padding: 16,
          height: "100%",
          overflow: "auto",
          background: "var(--surface-app)",
          color: "var(--text-primary)",
          fontFamily: "var(--font-mono)",
          fontSize: "var(--text-xs)",
          minHeight: 0,
        }}
      >
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 12 }}>
          <strong style={{ fontSize: "var(--text-sm)", color: "var(--severity-critical)" }}>
            {this.props.label ? `${this.props.label}: error` : "Error"}
          </strong>
          <button
            type="button"
            onClick={this.reset}
            style={{
              background: "var(--surface-card)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-strong)",
              borderRadius: 6,
              padding: "4px 10px",
              cursor: "pointer",
              fontSize: "var(--text-xs)",
            }}
          >
            Retry
          </button>
        </div>
        <div style={{ whiteSpace: "pre-wrap", color: "var(--severity-critical)", marginBottom: 12 }}>
          {error.message || String(error)}
        </div>
        {error.stack ? (
          <details style={{ marginBottom: 12 }}>
            <summary style={{ cursor: "pointer", color: "var(--text-secondary)" }}>Stack</summary>
            <pre style={{ whiteSpace: "pre-wrap", color: "var(--text-secondary)", fontSize: "var(--text-xs)", lineHeight: 1.4 }}>
              {error.stack}
            </pre>
          </details>
        ) : null}
        {info?.componentStack ? (
          <details>
            <summary style={{ cursor: "pointer", color: "var(--text-secondary)" }}>Component stack</summary>
            <pre style={{ whiteSpace: "pre-wrap", color: "var(--text-secondary)", fontSize: "var(--text-xs)", lineHeight: 1.4 }}>
              {info.componentStack}
            </pre>
          </details>
        ) : null}
      </div>
    );
  }
}
