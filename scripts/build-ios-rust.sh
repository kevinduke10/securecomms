#!/usr/bin/env bash
# Xcode build phase: build each selected architecture for THIS SDK, then stage
# one library beside the Xcode products. Device and simulator slices never mix.
set -euo pipefail
cd "$(dirname "$0")/.."
repo_root="$PWD"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
command -v cargo >/dev/null || { echo 'error: Install Rust with rustup before building Cirra.' >&2; exit 1; }
: "${PLATFORM_NAME:?Run this script from the Cirra Xcode target.}"
: "${ARCHS:?Xcode did not provide its target architectures.}"
: "${BUILT_PRODUCTS_DIR:?Xcode did not provide its products directory.}"
case "$PLATFORM_NAME" in
  iphoneos|iphonesimulator) ;;
  *) echo "error: Cirra requires an iOS device or simulator destination (got $PLATFORM_NAME)." >&2; exit 1 ;;
esac
export SDKROOT="$(xcrun --sdk "$PLATFORM_NAME" --show-sdk-path)"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-16.0}"
export CARGO_TARGET_DIR="$repo_root/target/ios"
libraries=()
for architecture in $ARCHS; do
  case "$PLATFORM_NAME:$architecture" in
    iphoneos:arm64) rust_target=aarch64-apple-ios ;;
    iphonesimulator:arm64) rust_target=aarch64-apple-ios-sim ;;
    iphonesimulator:x86_64) rust_target=x86_64-apple-ios ;;
    *) echo "error: Unsupported iOS SDK/architecture: $PLATFORM_NAME/$architecture" >&2; exit 1 ;;
  esac
  echo "Building Cirra Rust library: $PLATFORM_NAME/$architecture ($rust_target)"
  rustup target add "$rust_target"
  cargo build --locked -p cirra-ios-bridge --release --target "$rust_target"
  libraries+=("$CARGO_TARGET_DIR/$rust_target/release/libcirra_ios_bridge.a")
done
staging="$BUILT_PRODUCTS_DIR/cirra-rust"
mkdir -p "$staging"
if [[ ${#libraries[@]} -eq 1 ]]; then
  cp "${libraries[0]}" "$staging/libcirra_ios_bridge.a.tmp"
else
  xcrun lipo -create "${libraries[@]}" -output "$staging/libcirra_ios_bridge.a.tmp"
fi
mv "$staging/libcirra_ios_bridge.a.tmp" "$staging/libcirra_ios_bridge.a"
xcrun lipo "$staging/libcirra_ios_bridge.a" -verify_arch $ARCHS
