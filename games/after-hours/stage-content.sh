#!/bin/sh
# Trunk post-build hook: stages the files listed in content-manifest.txt, and a
# registry trimmed to them, into the web build's content/ directory. Mirrors
# what build.rs does for native builds.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
source_root="$here/../../examples/tech-demo/content"
pack="UAL1"
out="${TRUNK_STAGING_DIR:-$here/dist}/content"

rm -rf "$out"
mkdir -p "$out/$pack"

files=$(grep -v '^[[:space:]]*#' "$here/content-manifest.txt" | awk 'NF >= 2 { print $2 }')
for file in $files; do
    cp "$source_root/$pack/$file" "$out/$pack/$file"
done

{
    grep -v '=' "$source_root/.registry.toml"
    for file in $files; do
        grep "= \"content/$pack/$file\"\$" "$source_root/.registry.toml"
    done
} > "$out/.registry.toml"
