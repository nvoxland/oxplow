#!/bin/sh
# Keep the browser suite's frontend up for local runs: build `dist/` once,
# rebuild it on every change under apps/desktop (`vite build --watch`), and
# serve it on this checkout's port. `bun run e2e …` then uses this server as
# it is (playwright.config.ts, `reuseExistingServer` off CI) and skips its own
# build and start. `vite preview` reads each file from disk per request, so a
# rebuilt `dist/` is what the next page load gets. Ctrl-C stops both.
set -eu
cd "$(dirname "$0")/.."
PORT=$(bun -e 'import { previewPort } from "./tests-e2e/support/port.ts"; console.log(previewPort(process.cwd(), process.env))')
bun run --cwd apps/desktop build
bun run --cwd apps/desktop vite build --watch --logLevel warn &
WATCH=$!
trap 'kill $WATCH 2>/dev/null' EXIT INT TERM
echo "e2e: serving apps/desktop/dist on http://127.0.0.1:$PORT (rebuilt on change)"
bun run --cwd apps/desktop preview -- --host 127.0.0.1 --port "$PORT" --strictPort
