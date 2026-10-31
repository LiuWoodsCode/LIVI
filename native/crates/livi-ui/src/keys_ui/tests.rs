// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};

use super::*;

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

fn flush() {
    slint::platform::update_timers_and_animations();
}

fn click(ui: &AppWindow) {
    let position = slint::LogicalPosition::new(300.0, 100.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    flush();
}

fn type_key(ui: &AppWindow, code: &str, text: slint::SharedString, config: &serde_json::Value) {
    assert_eq!(decide(code, context(ui, false), config), Outcome::Pass);
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    flush();
}

#[test]
fn pointer_focused_fields_receive_typing_and_backspace() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let config = serde_json::json!({ "bindings": {
        "back": "Backspace", "cycleSession": "Digit7", "playPause": "Digit8"
    } });

    for screen in [ScreenKind::Text, ScreenKind::Color, ScreenKind::Number] {
        for initial_zone in [FocusZone::None, FocusZone::Nav, FocusZone::Main] {
            let ui = AppWindow::new().unwrap();
            ui.window().set_size(slint::PhysicalSize::new(1280, 720));
            ui.set_page(Page::Settings);
            let settings = ui.global::<SettingsState>();
            settings.set_kind(screen);
            settings.set_can_back(true);
            settings.set_number_raw("12".into());
            let committed = Rc::new(RefCell::new(slint::SharedString::default()));
            let record = {
                let committed = committed.clone();
                move |text| *committed.borrow_mut() = text
            };
            match screen {
                ScreenKind::Text => settings.on_text_commit(record),
                ScreenKind::Color => settings.on_color_hex_commit(record),
                ScreenKind::Number => settings.on_number_commit(record),
                _ => unreachable!(),
            }
            let focus = ui.global::<Focus>();
            focus.set_zone(initial_zone);
            // The header's Back button is initially selected, not the input.
            focus.set_main_index(0);
            ui.show().unwrap();
            flush();

            click(&ui);
            let ctx = context(&ui, false);
            assert_eq!(ctx.zone, Zone::Main, "{screen:?}, {initial_zone:?}");
            assert!(ctx.editing, "{screen:?}, {initial_zone:?}");
            assert_eq!(
                ctx.focused,
                if screen == ScreenKind::Number {
                    Focused::Number
                } else {
                    Focused::Text
                }
            );
            type_key(&ui, "Digit7", "7".into(), &config);
            type_key(&ui, "Digit8", "8".into(), &config);
            type_key(
                &ui,
                "Backspace",
                slint::platform::Key::Backspace.into(),
                &config,
            );
            assert_eq!(
                decide("Enter", context(&ui, false), &config),
                Outcome::StopEditing
            );
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: slint::platform::Key::Return.into(),
            });
            assert_eq!(&*committed.borrow(), "7");

            text_done(&ui);
            flush();
            assert!(!context(&ui, false).editing);
            // Re-entering with the pointer must restore editing after focus was cleared.
            click(&ui);
            assert!(context(&ui, false).editing);
            assert_eq!(
                decide("Escape", context(&ui, false), &config),
                Outcome::StopEditing
            );
            ui.hide().unwrap();
        }
    }
}
