#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <tag> <repo> <dist_dir>" >&2
  echo "example: $0 v0.1.0 AmirTlinov/context_pack dist" >&2
  exit 1
fi

TAG="$1"
REPO="$2"
DIST_DIR="$3"

version="${TAG#v}"
base_url="https://github.com/${REPO}/releases/download/${TAG}"

checksums_file="${DIST_DIR}/checksums.sha256"
if [[ ! -f "$checksums_file" ]]; then
  echo "error: checksums file not found: ${checksums_file}" >&2
  exit 1
fi

sha_for() {
  local artifact="$1"
  local sha
  sha="$(grep -E "[[:space:]]${artifact}\$" "$checksums_file" | awk '{print $1}' | head -n1 || true)"
  if [[ -z "$sha" ]]; then
    echo "error: missing sha256 for ${artifact} in ${checksums_file}" >&2
    exit 1
  fi
  echo "$sha"
}

linux_x64_sha="$(sha_for "mcp-context-pack-x86_64-unknown-linux-gnu.tar.gz")"
linux_arm64_sha="$(sha_for "mcp-context-pack-aarch64-unknown-linux-gnu.tar.gz")"
mac_x64_sha="$(sha_for "mcp-context-pack-x86_64-apple-darwin.tar.gz")"
mac_arm64_sha="$(sha_for "mcp-context-pack-aarch64-apple-darwin.tar.gz")"
win_x64_sha="$(sha_for "mcp-context-pack-x86_64-pc-windows-msvc.zip")"

cat > "${DIST_DIR}/mcp-context-pack.rb" <<EOF
class McpContextPack < Formula
  desc "High-signal MCP context handoff for multi-agent coding"
  homepage "https://github.com/${REPO}"
  version "${version}"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "${base_url}/mcp-context-pack-aarch64-apple-darwin.tar.gz"
      sha256 "${mac_arm64_sha}"
    else
      url "${base_url}/mcp-context-pack-x86_64-apple-darwin.tar.gz"
      sha256 "${mac_x64_sha}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "${base_url}/mcp-context-pack-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "${linux_arm64_sha}"
    else
      url "${base_url}/mcp-context-pack-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${linux_x64_sha}"
    end
  end

  def install
    bin.install "mcp-context-pack"
  end

  test do
    assert_predicate bin/"mcp-context-pack", :exist?
  end
end
EOF

cat > "${DIST_DIR}/mcp-context-pack.json" <<EOF
{
  "version": "${version}",
  "description": "High-signal MCP context handoff for multi-agent coding",
  "homepage": "https://github.com/${REPO}",
  "license": "MIT",
  "architecture": {
    "64bit": {
      "url": "${base_url}/mcp-context-pack-x86_64-pc-windows-msvc.zip",
      "hash": "${win_x64_sha}"
    }
  },
  "bin": "mcp-context-pack.exe"
}
EOF

echo "generated:"
echo "  ${DIST_DIR}/mcp-context-pack.rb"
echo "  ${DIST_DIR}/mcp-context-pack.json"
