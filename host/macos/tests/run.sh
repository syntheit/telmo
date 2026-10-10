#!/bin/sh
# Compiles the host files that have tests (BluetoothGuard, ClipboardItem, RebuildStatus, EqualizerDSP, ClockSchedule, Icons) with their tests and runs them.
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
swiftc -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -o "$work/island-tests" "$here/../RebuildStatus.swift" "$here/island_tests.swift"
"$work/island-tests"
swiftc -O -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -o "$work/equalizer-tests" "$here/../EqualizerDSP.swift" "$here/equalizer_tests.swift"
"$work/equalizer-tests"
swiftc -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -o "$work/clock-tests" "$here/../ClockSchedule.swift" "$here/clock_tests.swift"
"$work/clock-tests"
swiftc -swift-version 5 -parse-as-library -sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk \
  -framework AppKit -o "$work/icons-tests" "$here/../Icons.swift" "$here/icons_tests.swift"
"$work/icons-tests"
