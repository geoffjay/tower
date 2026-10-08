#!/bin/bash
# Generate the AUR PKGBUILD (or .SRCINFO) for a tower release by filling in
# the version and per-target sha256 checksums from the release's checksums.txt.
#
# Usage:
#   ./scripts/gen-aur-pkgbuild.sh <version> [checksums.txt] [--srcinfo] > PKGBUILD|.SRCINFO
#
#   version         Release version (leading 'v' optional, e.g. v0.1.0)
#   checksums.txt   Path to the release checksums file
#                   (default: downloaded from the GitHub release for <version>)
#   --srcinfo       Emit .SRCINFO instead of PKGBUILD. Rendered from the same
#                   version/URL/checksum values so the two files cannot drift;
#                   avoids needing makepkg (unavailable on ubuntu runners and
#                   macOS).
#
# The rendered file is written to stdout so the caller decides where it lands
# (i.e. the aur@aur.archlinux.org:tower-bin.git checkout). The checked-in
# template packaging/aur/PKGBUILD.tpl is never modified.
#
# The checksums file is the `sha256sum *.tar.gz` output produced by the
# release workflow, with lines of the form "<sha256>  <filename>".
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REPO="geoffjay/tower"
TEMPLATE="$PROJECT_ROOT/packaging/aur/PKGBUILD.tpl"

VERSION=""
CHECKSUMS=""
SRCINFO=0
for arg in "$@"; do
    case "$arg" in
        --srcinfo) SRCINFO=1 ;;
        -*) echo "error: unknown flag $arg" >&2; exit 2 ;;
        *)
            if [[ -z "$VERSION" ]]; then
                VERSION="$arg"
            elif [[ -z "$CHECKSUMS" ]]; then
                CHECKSUMS="$arg"
            else
                echo "Usage: $0 <version> [checksums.txt] [--srcinfo]" >&2
                exit 2
            fi
            ;;
    esac
done
if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [checksums.txt] [--srcinfo]" >&2
    exit 1
fi
VERSION="${VERSION#v}"

# Download checksums.txt from the release if not provided locally.
CLEANUP=""
if [[ -z "$CHECKSUMS" ]]; then
    CHECKSUMS="$(mktemp)"
    CLEANUP="$CHECKSUMS"
    trap 'rm -f "$CLEANUP"' EXIT
    url="https://github.com/${REPO}/releases/download/v${VERSION}/checksums.txt"
    echo "Downloading $url" >&2
    curl -fsSL "$url" -o "$CHECKSUMS"
fi

sum_for() {
    local name="$1"
    local sum
    sum="$(grep " ${name}\$" "$CHECKSUMS" | awk '{print $1}' | head -1)"
    if [[ -z "$sum" ]]; then
        echo "error: no checksum for $name in $CHECKSUMS" >&2
        exit 1
    fi
    echo "$sum"
}

# Single source of values for both renders.
SHA_LNX_ARM="$(sum_for "tower-aarch64-unknown-linux-gnu.tar.gz")"
SHA_LNX_X86="$(sum_for "tower-x86_64-unknown-linux-gnu.tar.gz")"
URL_LNX_ARM="https://github.com/${REPO}/releases/download/v${VERSION}/tower-aarch64-unknown-linux-gnu.tar.gz"
URL_LNX_X86="https://github.com/${REPO}/releases/download/v${VERSION}/tower-x86_64-unknown-linux-gnu.tar.gz"

if [[ "$SRCINFO" -eq 1 ]]; then
    cat <<EOF
pkgbase = tower-bin
pkgdesc = Control and visibility for herds of coding agents
pkgver = ${VERSION}
pkgrel = 1
url = https://github.com/${REPO}
arch = aarch64
arch = x86_64
license = MIT
license = Apache-2.0
provides = tower
conflicts = tower
conflicts = tower-git
source_aarch64 = ${URL_LNX_ARM}
sha256sums_aarch64 = ${SHA_LNX_ARM}
source_x86_64 = ${URL_LNX_X86}
sha256sums_x86_64 = ${SHA_LNX_X86}
EOF
    echo "Rendered .SRCINFO for v${VERSION}" >&2
else
    sed \
        -e "s|@@URL_AARCH64@@|${URL_LNX_ARM}|g" \
        -e "s|@@URL_X86_64@@|${URL_LNX_X86}|g" \
        -e "s|@@SHA256_AARCH64@@|${SHA_LNX_ARM}|g" \
        -e "s|@@SHA256_X86_64@@|${SHA_LNX_X86}|g" \
        -e "s|@@VERSION@@|${VERSION}|g" \
        "$TEMPLATE"
    echo "Rendered PKGBUILD for v${VERSION}" >&2
fi
