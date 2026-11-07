#!/bin/sh
# `bun run test:fast [<nextest args>]`: nextest, after what the run needs.
#
# A run of only crates that build no host (none of them reaches
# oxplow-script, oxplow-app or oxplow-provider-fake, as a dependency or a
# dev-dependency) needs no sidecars and no fake provider: it skips
# `test-prereqs.sh` and starts in seconds. Any other run — a whole-workspace
# one, or one that names a host crate — gets the full prerequisites first.
#
# PURE is that set; adding a dependency on one of the three to a crate
# here means taking it out of the list.
set -e
cd "$(dirname "$0")/.."
PURE=" oxplow-agent-text oxplow-ai oxplow-ai-fake oxplow-ai-rig oxplow-code-deps \
oxplow-code-dup oxplow-code-metrics oxplow-config oxplow-coverage oxplow-db oxplow-domain \
oxplow-fs-watch oxplow-git oxplow-harness-fake oxplow-harnesses oxplow-lsp \
oxplow-lsp-installer oxplow-oauth-redirect oxplow-provider-protocol oxplow-pty \
oxplow-runtime oxplow-session oxplow-tasks "

# Whether every crate the args select is in PURE (no `-p` selects them all).
pure() {
  selected=0
  while [ $# -gt 0 ]; do
    case "$1" in
      -p | --package)
        shift
        case "$PURE" in *" $1 "*) selected=1 ;; *) return 1 ;; esac
        ;;
      -p* | --package=*)
        name="${1#--package=}"
        name="${name#-p}"
        case "$PURE" in *" $name "*) selected=1 ;; *) return 1 ;; esac
        ;;
      --workspace | --all) return 1 ;;
    esac
    shift
  done
  [ "$selected" = 1 ]
}

if pure "$@"; then
  sh scripts/sweep-target.sh
else
  sh scripts/test-prereqs.sh
fi
. scripts/test-tmpdir.sh
exec cargo nextest run "$@"
