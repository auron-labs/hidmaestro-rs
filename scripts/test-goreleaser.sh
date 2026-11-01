#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
cp "$repository_root/.goreleaser.yaml" "$repository_root/.gitignore" "$fixture/"
cd "$fixture"
git -c init.templateDir= init --quiet
git config user.name 'Release packaging test'
git config user.email 'release-test@example.invalid'
git add .
git commit --quiet -m 'test: release packaging fixture'
git tag v0.1.0
git remote add origin https://github.com/auron-labs/hidmaestro-rs.git

bundle=artifacts/stage/hidmaestro-rs-0.1.0-win-x64
mkdir -p "$bundle/runtime/nested"
for file in hidmaestro-mcp.exe hidmaestro-bridge.exe hidmaestro-bridge.dll HIDMaestro.Core.dll LICENSE LICENSE-HIDMAESTRO UNSIGNED.txt runtime/nested/payload.dll; do
    printf '%s\n' "$file" > "$bundle/$file"
done

# Exercise the real config without building native code or publishing anything.
GORELEASER_CURRENT_TAG=v0.1.0 goreleaser release --clean --skip=publish
archive=hidmaestro-rs-0.1.0-win-x64.zip
unzip -q "dist/$archive" -d extracted
diff -r "$bundle" "extracted/hidmaestro-rs-0.1.0-win-x64"
cd dist
sha256sum --check "$archive.sha256"
