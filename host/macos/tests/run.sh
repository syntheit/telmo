#!/bin/sh
# Compiles BluetoothGuard.swift with its tests and runs them.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
export DEVELOPER_DIR=/Library/Developer/CommandLineTools
unset NIX_CFLAGS_COMPILE NIX_LDFLAGS MACOSX_DEPLOYMENT_TARGET
export PATH=/Library/Developer/CommandLineTools/usr/bin:$PATH
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
swiftc -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -framework AppKit -framework IOBluetooth -o "$work/guard-tests" "$here/../BluetoothGuard.swift" "$here/guard_tests.swift"
"$work/guard-tests"
swiftc -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -framework AppKit -o "$work/clipboard-tests" "$here/../ClipboardItem.swift" "$here/clipboard_tests.swift"
"$work/clipboard-tests"
