#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "The iOS client requires macOS and full Xcode." >&2
  exit 1
fi
mode="${1:-simulator}"
case "$mode" in
  simulator)
    sdk=iphonesimulator
    if [[ "$(uname -m)" == arm64 ]]; then target=aarch64-apple-ios-sim; arch=arm64; else target=x86_64-apple-ios; arch=x86_64; fi
    destination='generic/platform=iOS Simulator'
    signing=(CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=-)
    ;;
  device) sdk=iphoneos; target=aarch64-apple-ios; arch=arm64; destination='generic/platform=iOS'; signing=(CODE_SIGNING_ALLOWED=NO) ;;
  *) echo 'Usage: scripts/build-ios.sh [simulator|device]' >&2; exit 1 ;;
esac
xcrun --sdk "$sdk" --show-sdk-path >/dev/null
rustup target add "$target"
if [[ ! -d node_modules ]]; then npm ci; fi
npm run build:ui
# Xcode builds and stages the Rust library for its selected SDK and architectures.
xcodebuild -project apps/ios/Cirra.xcodeproj -scheme Cirra -configuration Debug \
  -sdk "$sdk" -destination "$destination" -derivedDataPath "$PWD/target/ios/xcode" \
  ARCHS="$arch" "${signing[@]}" build
