// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::{env, fs, path::Path};

// Bare executables need their own identity too: a bundle's Info.plist is not
// available when running out/bin/LIVI and its helpers from a terminal.
pub fn embed_info_plist(binary: &str, identifier: &str, background: bool) {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let policy = if background {
        "<key>LSBackgroundOnly</key><true/>"
    } else {
        "<key>NSHighResolutionCapable</key><true/>"
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{identifier}</string>
<key>CFBundleName</key><string>{binary}</string>
<key>CFBundleExecutable</key><string>{binary}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
{policy}
<key>NSCameraUsageDescription</key><string>LIVI requires camera access for CarPlay.</string>
<key>NSMicrophoneUsageDescription</key><string>LIVI requires microphone access for voice control.</string>
<key>NSLocalNetworkUsageDescription</key><string>LIVI talks to a LIVI Link dongle on the local network.</string>
</dict></plist>
"#
    );
    let path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("Info.plist");
    fs::write(&path, plist).unwrap();
    // Separate linker arguments preserve paths containing spaces or commas.
    for arg in ["-sectcreate", "__TEXT", "__info_plist"] {
        println!("cargo:rustc-link-arg-bin={binary}=-Xlinker");
        println!("cargo:rustc-link-arg-bin={binary}={arg}");
    }
    println!("cargo:rustc-link-arg-bin={binary}=-Xlinker");
    println!("cargo:rustc-link-arg-bin={binary}={}", path.display());
}
