#!/bin/sh
# Fast fix loop for 32-bit Windows (Luiz, 2026-11-01): builds the windows-x86 openlaser.exe and openlaser-hw.exe on the
# Mac with cargo-xwin and puts them, with the files shipped next to them, in ~/Desktop/OpenLaser-x86-test for AnyDesk
# file transfer. The CI-built zip (release.yml) stays the real download.
#
# Laser safety (AGENTS.md §1): this is a `hw` build, so Luiz runs it himself, into his own lane (.target/hw). Agents
# never run it. It only compiles; it starts nothing.
#
# Needs: rustup's i686-pc-windows-msvc target, cargo-xwin (`cargo install --locked cargo-xwin`) and LLVM's lld-link
# (`brew install llvm`). The first run downloads Microsoft's C runtime and Windows SDK libraries (about 1 GB in
# ~/Library/Caches/cargo-xwin) and so accepts Microsoft's licence for them.
#
# Usage: sh packaging/windows/build-x86-on-mac.sh      (from a checkout of the branch to test)
set -eu

export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v lld-link >/dev/null 2>&1; then
    for llvm in /opt/homebrew/opt/llvm/bin /usr/local/opt/llvm/bin; do
        [ -x "$llvm/lld-link" ] && PATH="$llvm:$PATH"
    done
fi
command -v cargo-xwin >/dev/null || { echo "cargo-xwin is missing: cargo install --locked cargo-xwin" >&2; exit 1; }
command -v lld-link >/dev/null || { echo "lld-link is missing: brew install llvm" >&2; exit 1; }
rustup target add i686-pc-windows-msvc >/dev/null

repo=$(cd "$(dirname "$0")/../.." && pwd)
target=i686-pc-windows-msvc
export CARGO_TARGET_DIR="$HOME/AI/OpenLaser/.target/hw"
# Static C runtime, as in release.yml. Only for this target, so the lane's macOS builds keep their cache.
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static"

cd "$repo"
cargo xwin build --release --locked --target "$target" -p openlaser -p openlaser-hw --features openlaser/hw,openlaser-hw/hw

out="$HOME/Desktop/OpenLaser-x86-test"
mkdir -p "$out"
cp "$CARGO_TARGET_DIR/$target/release/openlaser.exe" "$CARGO_TARGET_DIR/$target/release/openlaser-hw.exe" "$out/"
# CRLF for cmd.exe and old Notepad, whatever this checkout's line endings are.
for f in packaging/README.txt "packaging/windows/Start OpenLaser with laser.cmd" \
         "packaging/windows/OpenLaser diagnostics.cmd" packaging/windows/OpenLaser-diagnostics.ps1; do
    perl -pe 's/\r?\n/\r\n/' "$f" > "$out/$(basename "$f")"
done

# Both programs must be 32-bit x86 (PE machine 0x014c) and must not need VCRUNTIME140.dll.
for exe in "$out/openlaser.exe" "$out/openlaser-hw.exe"; do
    machine=$(perl -e 'open F, "<:raw", $ARGV[0] or die; read F, $h, 4096; $o = unpack "V", substr($h, 60, 4); printf "%04x", unpack "v", substr($h, $o + 4, 2)' "$exe")
    [ "$machine" = 014c ] || { echo "$exe is not x86 (machine $machine)" >&2; exit 1; }
    if grep -qai 'vcruntime140.dll' "$exe"; then echo "$exe needs VCRUNTIME140.dll" >&2; exit 1; fi
done

echo "Copy these 6 files to the PC's OpenLaser folder (replace the old ones):"
ls -l "$out"
shasum -a 256 "$out"/*.exe
