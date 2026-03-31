#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND="$ROOT/crates/web/frontend"

usage() {
  echo "Usage: $0 [command]"
  echo ""
  echo "Commands:"
  echo "  all        Build frontend + Rust (default)"
  echo "  frontend   Build only the Vue frontend"
  echo "  rust       Build only Rust"
  echo "  run        Build everything then run the server"
  echo "  install    Build frontend + cargo install --path crates/cli"
  echo "  dev        Run Vite dev server (hot reload)"
  echo ""
}

build_frontend() {
  echo "==> Building frontend..."
  cd "$FRONTEND"
  npm install --silent
  npm run build
  echo "    Done -> crates/web/static/dist/"
}

build_rust() {
  echo "==> Building Rust..."
  cd "$ROOT"
  cargo build --release
  echo "    Done -> target/release/"
}

cmd="${1:-all}"

case "$cmd" in
  all)
    build_frontend
    build_rust
    echo ""
    echo "Build complete."
    ;;
  frontend)
    build_frontend
    ;;
  rust)
    build_rust
    ;;
  run)
    build_frontend
    build_rust
    echo "==> Starting server..."
    cd "$ROOT"
    ./target/release/agent007
    ;;
  install)
    build_frontend
    echo "==> Installing via cargo..."
    cd "$ROOT"
    cargo install --path crates/cli
    echo ""
    echo "Install complete. Run: agent007"
    ;;
  dev)
    echo "==> Starting Vite dev server (proxies API to localhost:8007)..."
    cd "$FRONTEND"
    npm run dev
    ;;
  help|--help|-h)
    usage
    ;;
  *)
    echo "Unknown command: $cmd"
    usage
    exit 1
    ;;
esac
