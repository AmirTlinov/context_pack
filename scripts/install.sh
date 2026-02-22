#!/usr/bin/env bash
set -euo pipefail

REPO="${CONTEXT_PACK_REPO:-AmirTlinov/context_pack}"
VERSION="${CONTEXT_PACK_VERSION:-latest}"
INSTALL_DIR="${CONTEXT_PACK_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="mcp-context-pack"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' is not installed" >&2
    exit 1
  fi
}

require_cmd curl
require_cmd tar
require_cmd sed

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
    return
  fi
  echo "error: sha256sum or shasum is required for checksum verification" >&2
  exit 1
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux) os="unknown-linux-gnu" ;;
    Darwin) os="apple-darwin" ;;
    *)
      echo "error: unsupported OS '$os' (supported: Linux, Darwin)" >&2
      exit 1
      ;;
  esac

  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *)
      echo "error: unsupported CPU '$arch' (supported: x86_64, aarch64/arm64)" >&2
      exit 1
      ;;
  esac

  echo "${arch}-${os}"
}

resolve_version() {
  if [[ "$VERSION" != "latest" ]]; then
    echo "$VERSION"
    return
  fi

  local api tag
  api="https://api.github.com/repos/${REPO}/releases/latest"
  tag="$(
    curl -fsSL "$api" \
      | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' \
      | head -n1
  )"

  if [[ -z "$tag" ]]; then
    echo "error: failed to resolve latest release tag for ${REPO}" >&2
    exit 1
  fi

  echo "$tag"
}

main() {
  local target tag archive_name checksums_name download_url checksums_url archive_path checksums_path extracted_bin
  local expected_sha actual_sha
  # global (not local): referenced by EXIT trap
  TMP_DIR=""

  target="$(detect_target)"
  tag="$(resolve_version)"
  archive_name="${BINARY_NAME}-${target}.tar.gz"
  checksums_name="checksums.sha256"
  download_url="https://github.com/${REPO}/releases/download/${tag}/${archive_name}"
  checksums_url="https://github.com/${REPO}/releases/download/${tag}/${checksums_name}"

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT

  archive_path="${TMP_DIR}/${archive_name}"
  checksums_path="${TMP_DIR}/${checksums_name}"

  echo "→ Downloading ${archive_name} (${tag}) from ${REPO}"
  curl -fL "$download_url" -o "$archive_path"
  curl -fL "$checksums_url" -o "$checksums_path"

  expected_sha="$(
    grep -E "[[:space:]]${archive_name}\$" "$checksums_path" \
      | awk '{print $1}' \
      | head -n1
  )"
  if [[ -z "$expected_sha" ]]; then
    echo "error: checksum for ${archive_name} not found in ${checksums_name}" >&2
    exit 1
  fi
  actual_sha="$(sha256_file "$archive_path")"
  if [[ "$actual_sha" != "$expected_sha" ]]; then
    echo "error: checksum mismatch for ${archive_name}" >&2
    echo "expected: ${expected_sha}" >&2
    echo "actual:   ${actual_sha}" >&2
    exit 1
  fi

  tar -xzf "$archive_path" -C "$TMP_DIR"
  extracted_bin="${TMP_DIR}/${BINARY_NAME}"

  if [[ ! -f "$extracted_bin" ]]; then
    echo "error: binary '${BINARY_NAME}' not found in archive ${archive_name}" >&2
    exit 1
  fi

  mkdir -p "$INSTALL_DIR"
  install -m 755 "$extracted_bin" "${INSTALL_DIR}/${BINARY_NAME}"

  echo "✓ Installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
  echo "  If needed, add to PATH:"
  echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
}

main "$@"
