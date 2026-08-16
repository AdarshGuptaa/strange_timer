#!/bin/sh
# StrangeTimer one-command installer (Linux / macOS).
#
#   curl --proto '=https' --tlsv1.2 -fsSL \
#     https://github.com/AdarshGuptaa/strange_timer/releases/latest/download/install.sh | sh
#
# Installs into ~/.local by default (no sudo), sets up PATH, shell
# completions, autostart and starts the daemon. Options:
#   --version <tag>       install a specific release (default: latest)
#   --install-root <dir>  install under <dir> (default: $HOME/.local)
#   --no-autostart        do not register/start the autostart service
#   --no-completions      do not install shell completions
#   --uninstall           remove the installed copy (keeps your data)
#   --purge-data          with --uninstall: also delete timer data

set -eu

REPO="AdarshGuptaa/strange_timer"
VERSION=""
INSTALL_ROOT="${INSTALL_ROOT:-$HOME/.local}"
NO_AUTOSTART=0
NO_COMPLETIONS=0
UNINSTALL=0
PURGE_DATA=0

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 ;;
        --install-root) INSTALL_ROOT="${2:-}"; shift 2 ;;
        --no-autostart) NO_AUTOSTART=1; shift ;;
        --no-completions) NO_COMPLETIONS=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        --purge-data) PURGE_DATA=1; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

say() { printf '\033[1;36m%s\033[0m\n' "$*"; }
# curl flags: HTTPS-only by default; override with CURL_FLAGS for local
# testing against an http:// mirror. (No embedded quotes: CURL_FLAGS is
# word-split, so `'=https'` would reach curl as a literal quoted value.)
CURL_FLAGS="${CURL_FLAGS:---proto =https --tlsv1.2}"
die() { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# --- platform detection -----------------------------------------------------
uname_s="$(uname -s)"
case "$uname_s" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *) die "unsupported platform: $uname_s (use install.ps1 on Windows)" ;;
esac

uname_m="$(uname -m)"
case "$uname_m" in
    x86_64|amd64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) die "unsupported architecture: $uname_m" ;;
esac

if [ "$PLATFORM" = "macos" ] && [ "$ARCH" = "x86_64" ]; then
    die "macOS x86_64 builds are no longer published (GitHub Actions retired \
the Intel macOS runners) — use an Apple Silicon machine or the macos-aarch64 archive"
fi

BIN_DIR="$INSTALL_ROOT/bin"
PAYLOAD_ROOT="$INSTALL_ROOT/lib/strangetimer"

# --- uninstall --------------------------------------------------------------
if [ "$UNINSTALL" = "1" ]; then
    say "Uninstalling StrangeTimer…"
    if command -v strangetimer >/dev/null 2>&1; then
        "strangetimer" daemon stop 2>/dev/null || true
        "strangetimer" daemon uninstall 2>/dev/null || true
    fi
    rm -f "$BIN_DIR/strangetimer" "$BIN_DIR/strangetimer-daemon"
    rm -f "$HOME/.local/share/bash-completion/completions/strangetimer"
    rm -f "$HOME/.config/fish/completions/strangetimer.fish"
    rm -f "$HOME/.zfunc/_strangetimer"
    rm -rf "$PAYLOAD_ROOT"
    if [ "$PURGE_DATA" = "1" ]; then
        rm -rf "${STRANGETIMER_DATA_DIR:-$HOME/.local/share/strangetimer}"
        say "Timer data purged."
    else
        say "Your timer data was kept in ~/.local/share/strangetimer."
    fi
    say "Done. Open a new terminal for PATH changes to take effect."
    exit 0
fi

# --- resolve version --------------------------------------------------------
if [ -z "$VERSION" ]; then
    say "Resolving the latest release…"
    VERSION="$(curl $CURL_FLAGS -fsSL \
        "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
    # /releases/latest ignores prereleases; fall back to the most recent
    # release (which may be a prerelease) so a beta-only repo still installs.
    if [ -z "$VERSION" ]; then
        VERSION="$(curl $CURL_FLAGS -fsSL \
            "https://api.github.com/repos/$REPO/releases?per_page=1" 2>/dev/null \
            | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"
    fi
    [ -n "$VERSION" ] || die "could not determine the latest version"
fi
say "Installing StrangeTimer $VERSION ($PLATFORM-$ARCH) into $INSTALL_ROOT"

ARCHIVE="strangetimer-$VERSION-$PLATFORM-$ARCH.tar.gz"
BASE="${ST_URL_BASE:-https://github.com/$REPO/releases/download/$VERSION}"
URL="$BASE/$ARCHIVE"
SUMS_URL="$BASE/checksums.txt"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

say "Downloading $ARCHIVE…"
curl $CURL_FLAGS -fsSL -o "$TMP/$ARCHIVE" "$URL"

if command -v sha256sum >/dev/null 2>&1; then
    say "Verifying SHA-256…"
    if curl $CURL_FLAGS -fsSL -o "$TMP/checksums.txt" "$SUMS_URL" 2>/dev/null; then
        ( cd "$TMP" && sha256sum -c checksums.txt 2>/dev/null ) \
            || ( cd "$TMP" && grep -q "$(sha256sum "$ARCHIVE" | cut -d' ' -f1)" checksums.txt ) \
            || die "checksum verification failed — refusing to install"
    else
        echo "  (no checksums file published for $VERSION; skipping verification)" >&2
    fi
fi

say "Extracting…"
tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
[ -f "$TMP/strangetimer" ] || die "archive is missing the strangetimer binary"
[ -f "$TMP/strangetimer-daemon" ] || die "archive is missing the daemon binary"
[ -f "$TMP/assets/default.mp4" ] || die "archive is missing assets/default.mp4"

# --- install atomically -----------------------------------------------------
DEST="$PAYLOAD_ROOT/$VERSION"
mkdir -p "$PAYLOAD_ROOT" "$BIN_DIR"
rm -rf "$DEST.tmp"
mkdir -p "$DEST.tmp"
cp "$TMP/strangetimer" "$TMP/strangetimer-daemon" "$DEST.tmp/"
cp -r "$TMP/assets" "$DEST.tmp/"
mv "$DEST.tmp" "$DEST"
rm -f "$PAYLOAD_ROOT/current"
ln -s "$DEST" "$PAYLOAD_ROOT/current"
ln -sf "$PAYLOAD_ROOT/current/strangetimer" "$BIN_DIR/strangetimer"
ln -sf "$PAYLOAD_ROOT/current/strangetimer-daemon" "$BIN_DIR/strangetimer-daemon"
chmod +x "$BIN_DIR/strangetimer" "$BIN_DIR/strangetimer-daemon"

# --- PATH -------------------------------------------------------------------
PATH_LINE='export PATH="$HOME/.local/bin:$PATH"'
case "$PATH" in
    *"$BIN_DIR"*) : ;;
    *)
        if [ "$PLATFORM" = "linux" ]; then
            for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
                if [ -f "$rc" ] && ! grep -qsF "$BIN_DIR" "$rc"; then
                    printf '\n# StrangeTimer\n%s\n' "$PATH_LINE" >> "$rc"
                    echo "  added PATH entry to $rc"
                fi
            done
        else
            for rc in "$HOME/.zprofile" "$HOME/.zshrc" "$HOME/.profile"; do
                if [ -f "$rc" ] && ! grep -qsF "$BIN_DIR" "$rc"; then
                    printf '\n# StrangeTimer\n%s\n' "$PATH_LINE" >> "$rc"
                    echo "  added PATH entry to $rc"
                    break
                fi
            done
        fi
        ;;
esac

# Current shell cannot see the new PATH — use the full path from here on.
CLI="$BIN_DIR/strangetimer"
[ -x "$CLI" ] || die "install succeeded but $CLI is not executable"

# --- completions ------------------------------------------------------------
if [ "$NO_COMPLETIONS" = "1" ]; then
    echo "  (completions skipped)"
else
    say "Installing shell completions…"
    "$CLI" install-completions || echo "  (completion install reported a warning)"
fi

# --- autostart + daemon -----------------------------------------------------
if [ "$NO_AUTOSTART" = "1" ]; then
    say "Installed. Start the daemon later with: strangetimer daemon start"
else
    say "Registering autostart and starting the daemon…"
    "$CLI" daemon start || die "daemon failed to start"
fi

# --- health check -----------------------------------------------------------
say "Health check…"
"$CLI" --version >/dev/null || die "CLI does not run"
"$CLI" doctor >/dev/null || echo "  (doctor reported warnings — see `strangetimer doctor`)"

say ""
say "StrangeTimer $VERSION installed!"
say "  Open a NEW terminal, then try:"
say "    strangetimer create timer demo 1m"
say "    strangetimer run demo"
say "    strangetimer view timers"
say "Uninstall with: $0 --uninstall   (keep data) / --purge-data (wipe data)"
