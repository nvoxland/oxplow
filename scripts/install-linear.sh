#!/usr/bin/env bash
# Install the Linear provider example into a project (P7.A5,
# `.context/providers.md` "The Linear provider").
#
#   scripts/install-linear.sh <project>
#
# Builds oxplow-provider-linear (release) and copies
# examples/extensions/linear into <project>/oxplow/extensions/linear with
# the binary at its entry, bin/oxplow-provider-linear. The binary lives
# in the extension folder because consent hashes the folder: what you
# approve on Settings → Data → Programs is exactly what runs. Re-running
# replaces the folder, so a rebuilt binary shows up unapproved again.
set -euo pipefail

project="${1:?usage: scripts/install-linear.sh <project>}"
repo="$(cd "$(dirname "$0")/.." && pwd)"
[ -d "$project" ] || { echo "no project at $project" >&2; exit 2; }

cargo build --quiet --release --manifest-path "$repo/Cargo.toml" -p oxplow-provider-linear
target="$project/oxplow/extensions/linear"
rm -rf "$target"
mkdir -p "$(dirname "$target")"
cp -R "$repo/examples/extensions/linear" "$target"
cp "$repo/target/release/oxplow-provider-linear" "$target/bin/oxplow-provider-linear"
echo "Installed $target"
echo "Next: enable \`linear\` on Settings → Extensions, approve its program on"
echo "Settings → Data → Programs, then on Settings → Integrations set"
echo "LINEAR_API_KEY and the team, Check and Enable."
