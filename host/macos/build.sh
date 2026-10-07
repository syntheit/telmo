#!/bin/sh
# Builds Telmo.app with the Command Line Tools swiftc (no SwiftPM).
# Usage: SWIFTTERM_SRC=<SwiftTerm checkout> OUT=<dir> build.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
: "${SWIFTTERM_SRC:?path to a SwiftTerm checkout}"
: "${OUT:?output directory}"

# NSGlassEffectView needs the macOS 26+ SDK; ignore any SDKROOT from a Nix shell.
SDKROOT=${TELMO_SDK:-/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk}
[ -d "$SDKROOT" ] || SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk
export DEVELOPER_DIR=/Library/Developer/CommandLineTools
unset MACOSX_DEPLOYMENT_TARGET NIX_CFLAGS_COMPILE NIX_LDFLAGS
export PATH=/Library/Developer/CommandLineTools/usr/bin:$PATH

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat > "$work/BuildInfo.swift" <<'SWIFT'
enum SwiftTermBuildInfo { static let version = "1.19.0"; static let tag: String? = "v1.19.0"; static let branch: String? = nil }
SWIFT

app="$OUT/Telmo.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"

find "$SWIFTTERM_SRC/Sources/SwiftTerm" -name '*.swift' -not -path '*/iOS/*' -not -path '*Documentation.docc*' > "$work/terms"

# swiftc treats the file named main.swift as the entry point.
swiftc -O -swift-version 5 -sdk "$SDKROOT" -target arm64-apple-macos14.2 \
  -module-name Telmo \
  -framework AppKit -framework AVFoundation -framework Carbon -framework CoreAudio -framework IOBluetooth -framework CoreLocation -framework CoreWLAN \
  -framework LocalAuthentication -framework Security \
  -o "$app/Contents/MacOS/Telmo" \
  "$here"/*.swift "$work/BuildInfo.swift" @"$work/terms"

cp "$here/Info.plist" "$app/Contents/Info.plist"
