#!/bin/sh
# howmuchiai installer — downloads the latest release binary for your platform
# Usage: curl -sSL https://raw.githubusercontent.com/priyanshu-09/howmuchiai/main/install.sh | sh

set -e

REPO="priyanshu-09/howmuchiai"
BINARY_NAME="howmuchiai"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)
        case "$ARCH" in
            arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
            x86_64)        TARGET="x86_64-apple-darwin" ;;
            *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
        esac
        ;;
    Linux)
        case "$ARCH" in
            aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
            x86_64)        TARGET="x86_64-unknown-linux-gnu" ;;
            *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
        esac
        ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

# Get the latest release tag
LATEST=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)

if [ -z "$LATEST" ]; then
    echo "Error: Could not find latest release. Build from source instead:"
    echo "  git clone https://github.com/${REPO}.git && cd howmuchiai && cargo build --release"
    exit 1
fi

URL="https://github.com/${REPO}/releases/download/${LATEST}/${BINARY_NAME}-${TARGET}.tar.gz"

echo "Downloading howmuchiai ${LATEST} for ${TARGET}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -sSL "$URL" -o "$TMPDIR/howmuchiai.tar.gz"
tar xzf "$TMPDIR/howmuchiai.tar.gz" -C "$TMPDIR"

if [ -w "$INSTALL_DIR" ]; then
    mv "$TMPDIR/$BINARY_NAME" "$INSTALL_DIR/"
else
    echo "Installing to $INSTALL_DIR (requires sudo)..."
    if ! sudo mv "$TMPDIR/$BINARY_NAME" "$INSTALL_DIR/"; then
        echo ""
        echo "❌ Install failed — sudo couldn't write to $INSTALL_DIR."
        echo ""
        echo "Options:"
        echo "  1) Re-run and enter your sudo password:"
        echo "     curl -sSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh"
        echo "  2) Install to a path you own (no sudo needed):"
        echo "     INSTALL_DIR=\"\$HOME/.local/bin\" curl -sSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh"
        echo "     (make sure \$HOME/.local/bin is on your PATH)"
        echo ""
        echo "Your existing howmuchiai binary (if any) was NOT replaced — running"
        echo "\`howmuchiai\` will still use the old version until install succeeds."
        exit 1
    fi
fi

chmod +x "$INSTALL_DIR/$BINARY_NAME"

echo ""
echo "✓ Installed howmuchiai to $INSTALL_DIR/$BINARY_NAME"
echo "Run it: howmuchiai"
echo ""
