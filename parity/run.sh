#!/usr/bin/env bash
# Replays every script with the C# runner and the Rust interpreter and compares outputs.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${OUT:-/tmp/openpdn-parity}"
cs="$here/csharp/bin/Release/net10.0/ParityRunner"
rs="$here/../target/release/openpdn-parity"
# Always compare freshly built runners (stale binaries hide differences).
(cd "$here/.." && cargo build --release -q -p openpdn-parity)
if command -v dotnet >/dev/null; then
  dotnet build "$here/csharp/ParityRunner.csproj" -c Release -nologo -v q --no-restore >/dev/null \
    || { echo "C# runner build failed (run: dotnet restore $here/csharp)"; exit 2; }
fi
rm -rf "$out"; mkdir -p "$out/cs" "$out/rs"
[ -d "$here/fixtures/png" ] || python3 "$here/fixtures/make-png.py"
[ -d "$here/fixtures/pdn" ] || python3 "$here/fixtures/make-pdn.py"
status=0
for s in "$here"/scripts/*.pdnscript; do
  n="$(basename "$s" .pdnscript)"
  "$cs" "$s" "$out/cs/$n"
  "$rs" "$s" "$out/rs/$n"
  echo "== $n"
  "$rs" --compare "$out/cs/$n" "$out/rs/$n" || status=1
done
# Cross-engine .openpdn: each engine reads the document the other wrote.
x="$here/scripts/18-openpdn-read.pdnscript.in"
OPENPDN_XDOC="$out/rs/18-openpdn-write/doc.openpdn" "$cs" "$x" "$out/cs-reads-rs"
OPENPDN_XDOC="$out/cs/18-openpdn-write/doc.openpdn" "$rs" "$x" "$out/rs-reads-cs"
echo "== C# reads Rust .openpdn"
"$rs" --compare "$out/cs/18-openpdn-write" "$out/cs-reads-rs" || status=1
echo "== Rust reads C# .openpdn"
"$rs" --compare "$out/cs/18-openpdn-write" "$out/rs-reads-cs" || status=1
exit $status
