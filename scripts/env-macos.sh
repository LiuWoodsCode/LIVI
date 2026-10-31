# Source from the repository root: source scripts/env-macos.sh
# Prefer Homebrew tools over devkitPro's pkg-config.
export PATH="/opt/homebrew/bin:$PATH"
export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:/opt/homebrew/share/pkgconfig:/opt/homebrew/opt/libepoxy/lib/pkgconfig"
export LDFLAGS="-L/opt/homebrew/lib"
export CPPFLAGS="-I/opt/homebrew/include"

# The official framework supplies GStreamer headers, libraries, and plugins.
export PATH="/Library/Frameworks/GStreamer.framework/Versions/1.0/bin:$PATH"
export PKG_CONFIG_PATH="/Library/Frameworks/GStreamer.framework/Versions/1.0/lib/pkgconfig:$PKG_CONFIG_PATH"
