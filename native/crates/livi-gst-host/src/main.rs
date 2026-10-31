// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

#[cfg(target_os = "macos")]
mod audio_devices;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "macos")]
    if args.first().is_some_and(|a| a == "--audio-devices") {
        if let Err(e) = audio_devices::run(&args[1..]) {
            eprintln!("[audio] {e}");
            std::process::exit(1);
        }
        return;
    }
    if args.first().is_some_and(|a| a == "--probe") {
        println!("{}", livi_gst_host::gst::probe_json());
        return;
    }

    livi_gst_host::process::run(
        args.first().map_or("", String::as_str),
        args.get(1).map_or("", String::as_str),
    );
}
