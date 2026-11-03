// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use crate::settings::node::{Label, Node};

pub fn schema() -> Node {
    Node::route(
        "radio",
        Label::new("settings.radio", "Radio"),
        vec![super::dab::schema()],
    )
    .icon("radio")
}
