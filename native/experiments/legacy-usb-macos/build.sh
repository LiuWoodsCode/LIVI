#!/bin/sh
set -eu
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
sdk=$(xcrun --sdk macosx --show-sdk-path)
output=${1:-/private/tmp/LIVILegacyUSBProbe.kext}
mkdir -p "$output/Contents/MacOS"
xcrun clang++ -arch arm64e -isysroot "$sdk" -std=c++17 \
  -mkernel -nostdlib -fno-exceptions -fno-rtti \
  -DKERNEL -DKERNEL_PRIVATE -DDRIVER_PRIVATE \
  -I"$sdk/System/Library/Frameworks/Kernel.framework/Headers" \
  -Wall -Wextra -Werror -Wno-unused-parameter -Wno-deprecated-declarations \
  -c "$base/EnableDeviceMode.cpp" -o "$output.o"
xcrun ld -arch arm64e -kext -undefined dynamic_lookup \
  -o "$output/Contents/MacOS/LIVILegacyUSBProbe" "$output.o"
cp "$base/Info.plist" "$output/Contents/Info.plist"
codesign --force --sign - "$output"
codesign --verify --strict "$output"
printf 'Built %s\n' "$output"
