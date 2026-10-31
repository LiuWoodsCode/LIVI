// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

#[path = "../../build_support/macos.rs"]
mod macos;

fn main() {
    macos::embed_info_plist("livi-gst-host", "dev.f-io.livi.gst-host", true);
}
