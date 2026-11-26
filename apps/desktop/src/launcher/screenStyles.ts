import type React from "react";

/// The look of the shell's own full-window screens (the launcher, the host's
/// pair and home screens): one card on the app surface.

export const rootStyle: React.CSSProperties = {
  height: "100vh",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
};

export const cardStyle: React.CSSProperties = {
  width: 560,
  maxWidth: "90vw",
  maxHeight: "85vh",
  display: "flex",
  flexDirection: "column",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 10,
  padding: 28,
  boxSizing: "border-box",
};

export const titleStyle: React.CSSProperties = {
  margin: 0,
  fontSize: 22,
  fontWeight: "var(--weight-bold)" as unknown as number,
};

export const subtitleStyle: React.CSSProperties = {
  margin: "4px 0 0",
  color: "var(--text-secondary)",
};

export const sectionHeaderStyle: React.CSSProperties = {
  margin: "20px 0 8px",
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: "var(--text-muted)",
};

export const doorsStyle: React.CSSProperties = {
  display: "flex",
  gap: 8,
};

export const doorButtonStyle: React.CSSProperties = {
  flex: 1,
  padding: "10px 14px",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: "var(--weight-medium)" as unknown as number,
};

/// New Project is the accented primary — the launcher's job for a
/// first-time user is getting a project made.
export const newProjectButtonStyle: React.CSSProperties = {
  ...doorButtonStyle,
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
  border: "none",
};

export const openProjectButtonStyle: React.CSSProperties = {
  ...doorButtonStyle,
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
};

export const listStyle: React.CSSProperties = {
  listStyle: "none",
  margin: 0,
  padding: 0,
  overflowY: "auto",
};

export const rowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "8px 4px",
  borderBottom: "1px solid var(--border-subtle)",
};

export const rowOpenButtonStyle: React.CSSProperties = {
  flex: 1,
  minWidth: 0,
  display: "flex",
  flexDirection: "column",
  alignItems: "flex-start",
  gap: 2,
  background: "transparent",
  border: "none",
  cursor: "pointer",
  textAlign: "left",
  padding: 0,
  color: "inherit",
};

export const rowTitleStyle: React.CSSProperties = {
  fontWeight: "var(--weight-medium)" as unknown as number,
  color: "var(--text-primary)",
};

export const rowPathStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
  maxWidth: "100%",
};

export const rowMetaStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexShrink: 0,
};

export const rowTimeStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
};

export const missingBadgeStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--severity-critical)",
  border: "1px solid var(--severity-critical)",
  borderRadius: 4,
  padding: "0 4px",
};

export const emptyStyle: React.CSSProperties = {
  color: "var(--text-muted)",
  padding: "12px 4px",
};

export const errorStyle: React.CSSProperties = {
  background: "var(--accent-soft-bg)",
  color: "var(--severity-critical)",
  border: "1px solid var(--severity-critical)",
  borderRadius: 6,
  padding: "8px 12px",
  marginBottom: 12,
  fontSize: "var(--text-xs)",
};

export const remoteFormStyle: React.CSSProperties = {
  display: "flex",
  gap: 8,
};

export const remoteInputStyle: React.CSSProperties = {
  flex: 1,
  minWidth: 0,
  padding: "8px 10px",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
};

export const remoteConnectButtonStyle: React.CSSProperties = {
  padding: "8px 14px",
  background: "var(--accent)",
  color: "var(--accent-on-accent)",
  border: "none",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: "var(--weight-medium)" as unknown as number,
};

export const remoteHintStyle: React.CSSProperties = {
  margin: "8px 0 12px",
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
  lineHeight: 1.5,
};
