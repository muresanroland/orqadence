#!/bin/sh
# Prints the license notices of every crate linked into the release binaries,
# for the THIRD-PARTY-LICENSES.txt the release workflow attaches:
#   scripts/third-party-licenses.sh > THIRD-PARTY-LICENSES.txt
set -eu

cargo fetch --locked >&2
echo "orqa includes the following third-party crates, under their own licenses."
cargo metadata --format-version 1 --locked \
  --filter-platform aarch64-apple-darwin --filter-platform x86_64-unknown-linux-gnu |
  jq -r '[.resolve.nodes[].id] as $used | .packages[]
    | select(.source != null and (.id | IN($used[])))
    | [.name, .version, (.manifest_path | rtrimstr("/Cargo.toml")), (.license // "unknown"), (.authors | join(", "))]
    | @tsv' |
  sort -u |
  while IFS="$(printf '\t')" read -r name version dir license authors; do
    printf '\n%s\n%s %s\nLicense: %s\n' "================================================================" "$name" "$version" "$license"
    files=$(find "$dir" -maxdepth 1 -type f \( -iname 'licen[cs]e*' -o -iname 'copying*' -o -iname 'notice*' \) | sort)
    for f in $files; do
      printf '\n--- %s ---\n\n' "${f##*/}"
      cat "$f"
    done
    [ -n "$files" ] && continue
    # ponytail: only MIT crates ship without a license file today; another
    # license with none prints just its name, add its text here if one appears.
    [ "$license" = MIT ] || continue
    cat <<MIT

Copyright (c) $authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
MIT
  done
