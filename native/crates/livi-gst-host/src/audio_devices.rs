// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::io::{self, Write};

use gst::prelude::*;
use gstreamer as gst;

// Keep the listing and change markers consumed by core's audio device parser,
// without gst-device-monitor's gst_macos_main / Cocoa application loop.
pub fn run(args: &[String]) -> Result<(), String> {
    gst::init().map_err(|e| e.to_string())?;
    let monitor = gst::DeviceMonitor::new();
    let follow = args.iter().any(|a| a == "-f");
    for class in args.iter().filter(|a| a.as_str() != "-f") {
        if !matches!(class.as_str(), "Audio/Sink" | "Audio/Source") {
            return Err(format!("unknown audio device class: {class}"));
        }
        monitor.add_filter(Some(class), None);
    }
    if args.iter().all(|a| a == "-f") {
        monitor.add_filter(Some("Audio/Sink"), None);
        monitor.add_filter(Some("Audio/Source"), None);
    }
    if follow {
        monitor.start().map_err(|e| e.to_string())?;
        println!("Device changed:");
        io::stdout().flush().map_err(|e| e.to_string())?;
        for message in monitor.bus().iter_timed(gst::ClockTime::NONE) {
            if matches!(
                message.view(),
                gst::MessageView::DeviceAdded(_)
                    | gst::MessageView::DeviceRemoved(_)
                    | gst::MessageView::DeviceChanged(_)
            ) {
                println!("Device changed:");
                io::stdout().flush().map_err(|e| e.to_string())?;
            }
        }
        monitor.stop();
    } else {
        for device in monitor.devices() {
            let Some(props) = device.properties() else {
                continue;
            };
            let Ok(id) = props.get::<String>("unique-id") else {
                continue;
            };
            let default = props.get::<bool>("is-default").unwrap_or(false);
            println!(
                "Device found:\n\tname : {}\n\tclass : {}\n\tunique-id = \"{id}\"\n\tis-default = {default}\n",
                device.display_name(),
                device.device_class(),
            );
        }
    }
    Ok(())
}
