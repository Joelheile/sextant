#!/usr/bin/env bash
# Fails if network code exists outside app/src/models.rs (plan.md §10).
set -euo pipefail
cd "$(dirname "$0")/.."

banned='reqwest|ureq|hyper|hyper-util|h2|h3|quinn|curl|curl-sys|isahc|surf|attohttpc|rustls|native-tls|tungstenite|tokio-tungstenite|socket2|websocket|tauri-plugin-(shell|http|updater|websocket|upload)'
# Desktop targets only: Tauri pulls reqwest for Android and iOS, which Sextant never builds.
targets='aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-gnu'
status=0

for target in $targets; do
  found=$(cargo tree --workspace --target "$target" -e normal --prefix none --format '{p}' | awk '{print $1}' | sort -u | grep -E -x "$banned" || true)
  if [ -n "$found" ]; then
    echo "network crates on $target:" $found >&2
    status=1
  fi
  if cargo tree --workspace --target "$target" -e features -i tokio 2>/dev/null | grep -q 'tokio feature "net"'; then
    echo "tokio has the net feature on $target" >&2
    status=1
  fi
done

hits=$(grep -rnE 'std::net|tokio::net|TcpStream|TcpListener|UdpSocket' --include='*.rs' core app/src | grep -v '^app/src/models.rs:' || true)
if [ -n "$hits" ]; then
  echo "network code outside app/src/models.rs:" >&2
  echo "$hits" >&2
  status=1
fi

exit $status
