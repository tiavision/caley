#!/usr/bin/env bash
# Writes one version into every place Caley keeps one: the backend crate, the frontend package and the app
# shell (its crate, which Tauri reads the app's version from, and its package), lockfiles included.
# Run by the Release workflow; locally: .github/scripts/set-version.sh 1.3.0
set -euo pipefail

version="${1:-}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: $0 <x.y.z | x.y.z-rc.1>" >&2
  exit 1
fi

root="$(cd "$(dirname "$0")/../.." && pwd)"

# A crate's [package] version and its own entry in Cargo.lock (no network, no resolve)
crate() {
  local dir="$1" name="$2"
  perl -0pi -e "s/(\[package\]\n(?:[^\[]*?\n)?version = )\"[^\"]*\"/\${1}\"$version\"/" "$dir/Cargo.toml"
  perl -0pi -e "s/(\nname = \"\Q$name\E\"\nversion = )\"[^\"]*\"/\${1}\"$version\"/" "$dir/Cargo.lock"
  grep -q "^version = \"$version\"" "$dir/Cargo.toml" || { echo "no version written to $dir/Cargo.toml" >&2; exit 1; }
  grep -Pzq "\nname = \"\Q$name\E\"\nversion = \"\Q$version\E\"" "$dir/Cargo.lock" || { echo "no version written to $dir/Cargo.lock" >&2; exit 1; }
}

crate "$root/backend" caley
crate "$root/native/src-tauri" caley-native

for dir in frontend native; do
  (cd "$root/$dir" && npm version "$version" --no-git-tag-version --allow-same-version >/dev/null)
done

echo "Caley $version"
