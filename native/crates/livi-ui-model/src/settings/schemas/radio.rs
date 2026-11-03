// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use crate::settings::node::{CustomPage, Label, Node};

pub fn schema() -> Node {
    Node::route(
        "radio",
        Label::new("settings.radio", "Radio"),
        vec![
            super::dab::schema(),
            Node::route(
                "internet-radio",
                Label::new("settings.internetRadio", "Internet Radio"),
                vec![
                    Node::text(
                        Label::new("settings.stationName", "Name"),
                        "internetRadioDraft.name",
                    )
                    .length(Some(1), Some(128))
                    .display_value()
                    .icon("radio")
                    .page(Label::new("settings.stationName", "Name")),
                    Node::text(
                        Label::new("settings.streamUrl", "Stream URL"),
                        "internetRadioDraft.url",
                    )
                    .url()
                    .length(Some(1), Some(4096))
                    .display_value()
                    .icon("radio")
                    .page(Label::new("settings.streamUrl", "Stream URL")),
                    Node::text(
                        Label::new("settings.stationDescription", "Description"),
                        "internetRadioDraft.description",
                    )
                    .length(None, Some(512))
                    .display_value()
                    .icon("radio")
                    .page(Label::new("settings.stationDescription", "Description")),
                    Node::custom(
                        Label::new("settings.internetRadio", "Internet Radio"),
                        "internetRadioDraft",
                        CustomPage::InternetRadio,
                    ),
                ],
            )
            .icon("radio"),
        ],
    )
    .icon("radio")
}
