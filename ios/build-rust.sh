#!/bin/sh
# Build the Rust core as a static library for the platform Xcode is building
# (PLATFORM_NAME iphoneos or iphonesimulator; iphoneos when run by hand) into
# target/ios-$PLATFORM_NAME/libsolid_sandbox.a. Needs the targets:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
set -eu
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
platform="${PLATFORM_NAME:-iphoneos}"
case "$platform" in
    iphoneos) targets="aarch64-apple-ios" ;;
    iphonesimulator) targets="aarch64-apple-ios-sim x86_64-apple-ios" ;;
    *) echo "unknown platform $platform" >&2; exit 1 ;;
esac
# Xcode's environment points SDKROOT and friends at the app's SDK; cargo's
# build scripts run on the Mac itself
unset SDKROOT
out="target/ios-$platform"
mkdir -p "$out"
libs=""
for t in $targets; do
    cargo rustc --release --lib --crate-type staticlib --target "$t"
    libs="$libs target/$t/release/libsolid_sandbox.a"
done
# one target: copy; two (simulator on Apple silicon and Intel): one fat library
# shellcheck disable=SC2086
if [ "$(echo $libs | wc -w)" -eq 1 ]; then
    cp $libs "$out/libsolid_sandbox.a"
else
    lipo -create $libs -output "$out/libsolid_sandbox.a"
fi
echo "built $out/libsolid_sandbox.a"