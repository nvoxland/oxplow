export type UiLogLevel = "debug" | "info" | "warn" | "error";

const CLIENT_ID_KEY = "newde-ui-client-id";

let installed = false;

export function installUiLogging(): void {
  if (installed) return;
  installed = true;

  const original = {
    log: console.log.bind(console),
    info: console.info.bind(console),
    warn: console.warn.bind(console),
    error: console.error.bind(console),
  };

  console.log = (...args: unknown[]) => {
    original.log(...args);
    void sendUiLog("info", "console.log", { args: args.map(serializeValue) });
  };
  console.info = (...args: unknown[]) => {
    original.info(...args);
    void sendUiLog("info", "console.info", { args: args.map(serializeValue) });
  };
  console.warn = (...args: unknown[]) => {
    original.warn(...args);
    void sendUiLog("warn", "console.warn", { args: args.map(serializeValue) });
  };
  console.error = (...args: unknown[]) => {
    original.error(...args);
    void sendUiLog("error", "console.error", { args: args.map(serializeValue) });
  };

  window.addEventListener("error", (event) => {
    void sendUiLog("error", "window.error", {
      message: event.message,
      filename: event.filename,
      lineno: event.lineno,
      colno: event.colno,
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    void sendUiLog("error", "window.unhandledrejection", {
      reason: serializeValue(event.reason),
    });
  });

  void sendUiLog("info", "ui logging installed", {
    clientId: getUiClientId(),
    href: location.href,
  });
}

export function logUi(level: UiLogLevel, message: string, context?: Record<string, unknown>): void {
  void sendUiLog(level, message, context);
}

export function getUiClientId(): string {
  try {
    const existing = sessionStorage.getItem(CLIENT_ID_KEY);
    if (existing) return existing;
    const id = globalThis.crypto?.randomUUID?.() ?? `client-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    sessionStorage.setItem(CLIENT_ID_KEY, id);
    return id;
  } catch {
    return `client-${Date.now()}`;
  }
}

async function sendUiLog(level: UiLogLevel, message: string, context?: Record<string, unknown>): Promise<void> {
  try {
    await fetch("/api/logs/ui", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        clientId: getUiClientId(),
        level,
        message,
        context,
        timestamp: new Date().toISOString(),
      }),
      keepalive: true,
    });
  } catch {}
}

function serializeValue(value: unknown): unknown {
  if (value instanceof Error) {
    return { name: value.name, message: value.message, stack: value.stack };
  }
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || value === null) {
    return value;
  }
  if (value === undefined) return "undefined";
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    return String(value);
  }
}
