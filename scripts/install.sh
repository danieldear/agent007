#!/usr/bin/env bash
set -euo pipefail

REPO="${AGENT007_REPO:-danieldear/agent007}"
VERSION="${AGENT007_VERSION:-latest}"
INSTALL_DIR="${AGENT007_INSTALL_DIR:-}"

usage() {
  cat <<USAGE
Install agent007 from GitHub Releases.

Usage:
  $0 [--version <tag>] [--install-dir <path>] [--repo <owner/name>]

Options:
  --version <tag>      Release tag (default: latest)
  --install-dir <dir>  Install directory (default: /usr/local/bin if writable, else ~/.local/bin)
  --repo <owner/name>  GitHub repository (default: danieldear/agent007)
  -h, --help           Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="$2"
      shift 2
      ;;
    --install-dir|--bin-dir)
      INSTALL_DIR="$2"
      shift 2
      ;;
    --repo)
      REPO="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ "$VERSION" == "latest" ]]; then
  API_URL="https://api.github.com/repos/${REPO}/releases/latest"
  TAG="$(curl -fsSL "$API_URL" | grep -m1 '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')"
  if [[ -z "$TAG" ]]; then
    echo "Could not determine latest release for ${REPO}" >&2
    exit 1
  fi
else
  TAG="$VERSION"
fi

if [[ "$TAG" != v* ]]; then
  TAG="v${TAG}"
fi

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64|amd64)
        TARGET="x86_64-unknown-linux-gnu"
        ;;
      *)
        echo "Unsupported Linux architecture: ${ARCH}" >&2
        echo "Currently supported: x86_64" >&2
        exit 1
        ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      x86_64)
        cat >&2 <<EOF
Intel macOS prebuilt artifacts are not currently published.
Use one of these options instead:
  1. Source install:
     cargo install --git https://github.com/${REPO}.git agent007
  2. Apple Silicon machine / runner using the aarch64 macOS release artifact
EOF
        exit 1
        ;;
      arm64|aarch64)
        TARGET="aarch64-apple-darwin"
        ;;
      *)
        echo "Unsupported macOS architecture: ${ARCH}" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "Unsupported operating system: ${OS}" >&2
    exit 1
    ;;
esac

if [[ -z "$INSTALL_DIR" ]]; then
  if [[ -w /usr/local/bin ]]; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="${HOME}/.local/bin"
  fi
fi

mkdir -p "$INSTALL_DIR"

ASSET="agent007-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Installing agent007 ${TAG} (${TARGET}) from ${REPO} ..."

if ! curl -fL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${ASSET}"; then
  if [[ "$OS" == "Darwin" ]]; then
    cat >&2 <<EOF
No prebuilt macOS artifact is published for ${TAG} right now.
Use source install instead:
  cargo install --git https://github.com/${REPO}.git agent007
EOF
  else
    echo "Failed to download ${ASSET} from ${BASE_URL}" >&2
  fi
  exit 1
fi

curl -fL "${BASE_URL}/SHA256SUMS" -o "${TMP_DIR}/SHA256SUMS"

checksum_line="$(grep " ${ASSET}$" "${TMP_DIR}/SHA256SUMS" || true)"
if [[ -z "$checksum_line" ]]; then
  echo "Checksum entry for ${ASSET} not found in SHA256SUMS" >&2
  exit 1
fi

expected_sha="$(awk '{print $1}' <<<"$checksum_line")"
if command -v sha256sum >/dev/null 2>&1; then
  actual_sha="$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
else
  actual_sha="$(shasum -a 256 "${TMP_DIR}/${ASSET}" | awk '{print $1}')"
fi

if [[ "$expected_sha" != "$actual_sha" ]]; then
  echo "Checksum mismatch for ${ASSET}" >&2
  echo "expected: ${expected_sha}" >&2
  echo "actual:   ${actual_sha}" >&2
  exit 1
fi

tar -xzf "${TMP_DIR}/${ASSET}" -C "$TMP_DIR"
install -m 0755 "${TMP_DIR}/agent007" "${INSTALL_DIR}/agent007"

echo "Installed: ${INSTALL_DIR}/agent007"

case ":$PATH:" in
  *":${INSTALL_DIR}:"*)
    ;;
  *)
    echo "${INSTALL_DIR} is not in PATH. Add this line to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac

echo "Run: agent007 --help"
