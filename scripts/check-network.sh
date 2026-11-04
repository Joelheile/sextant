#!/usr/bin/env bash
# Fails if network code exists outside app/src/models.rs and the updater plugin.
set -euo pipefail
cd "$(dirname "$0")/.."

# socket2 is missing: the updater's reqwest switches on tokio's net feature, which brings it in
# under tokio. The source check below catches any use of tokio::net.
banned='reqwest|ureq|hyper|hyper-util|h2|h3|quinn|curl|curl-sys|isahc|surf|attohttpc|rustls|native-tls|tungstenite|tokio-tungstenite|websocket|tauri-plugin-(shell|http|updater|websocket|upload)'
# Desktop targets only: Tauri pulls reqwest for Android and iOS, which Sextant never builds.
targets='aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-gnu'
status=0

for target in $targets; do
  # Pruning the updater hides the network crates it brings, so any left came in another way.
  found=$(cargo tree --workspace --target "$target" --prune tauri-plugin-updater -e normal,build --prefix none --format '{p}' | awk '{print $1}' | sort -u | grep -E -x "$banned" || true)
  if [ -n "$found" ]; then
    echo "network crates on $target:" $found >&2
    status=1
  fi
done

hits=$(grep -rnE 'std::net|tokio::net|TcpStream|TcpListener|UdpSocket|NSURLSession|URLSession' --include='*.rs' core app/src | grep -v '^app/src/models.rs:' || true)
if [ -n "$hits" ]; then
  echo "network code outside app/src/models.rs:" >&2
  echo "$hits" >&2
  status=1
fi

exit $status
