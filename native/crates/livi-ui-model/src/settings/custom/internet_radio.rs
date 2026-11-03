// SPDX-FileCopyrightText: 2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use super::dab;
use crate::i18n::Strings;
use crate::settings::view::{Change, Row, RowKind, Tool};
use livi_core_proto::{message::Action, state::InternetRadioDraft};
use serde_json::{Value, json};

pub fn rows(state: &Value, t: &Strings) -> Vec<Row> {
    let draft: InternetRadioDraft =
        serde_json::from_value(state["config"]["internetRadioDraft"].clone()).unwrap_or_default();
    let valid_url = (draft.url.trim().starts_with("https://")
        || draft.url.trim().starts_with("http://"))
        && draft
            .url
            .trim()
            .split("://")
            .nth(1)
            .is_some_and(|s| !s.is_empty());
    let clear = || Change::Config(json!({ "internetRadioDraft": InternetRadioDraft::default() }));
    let mut rows = vec![
        Row {
            kind: RowKind::Button,
            button: t.t(
                if draft.service_id.is_some() {
                    "settings.saveStation"
                } else {
                    "settings.addStation"
                },
                if draft.service_id.is_some() {
                    "Save station"
                } else {
                    "Add station"
                },
            ),
            enabled: !draft.name.trim().is_empty() && valid_url,
            action: Some(Action::InternetRadioSave { draft }),
            ..Row::default()
        },
        Row {
            kind: RowKind::Button,
            button: t.t("settings.newStation", "New station"),
            enabled: true,
            outlined: true,
            change: Some(clear()),
            ..Row::default()
        },
    ];
    let d = dab::dab(state);
    let station_rows = dab::stations(&d, t);
    for (st, mut row) in d.stations.iter().zip(station_rows) {
        let Some(internet) = &st.internet else {
            continue;
        };
        row.kind = RowKind::Button;
        row.button = t.t("settings.editStation", "Edit");
        row.outlined = true;
        row.change = Some(Change::Config(
            json!({ "internetRadioDraft": InternetRadioDraft {
            service_id: Some(st.service_id), name: st.label.clone(), url: internet.url.clone(), description: internet.description.clone(),
        } }),
        ));
        row.tools.push(Tool {
            icon: "CloseOutlined",
            label: t.t("settings.deleteStation", "Delete station"),
            on: false,
            tint: None,
            enabled: true,
            switch: false,
            action: Action::InternetRadioDelete {
                service_id: st.service_id,
            },
        });
        rows.push(row);
    }
    if let Some(error) = d.error {
        rows.push(super::info(t.t("settings.dabError", "Error"), None, error));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use livi_core_proto::state::{Dab, DabStation, InternetStation};

    #[test]
    fn saved_streams_offer_editing_and_deletion() {
        let station = DabStation {
            service_id: u32::MAX,
            label: "Online News".into(),
            short_label: String::new(),
            ensemble: String::new(),
            channel: String::new(),
            frequency_hz: 0,
            favorite: true,
            hidden: false,
            logo: None,
            internet: Some(InternetStation {
                url: "https://radio.example/live".into(),
                description: "News".into(),
            }),
        };
        let state = json!({ "dab": Dab { stations: vec![station], ..Default::default() }, "config": { "internetRadioDraft": InternetRadioDraft::default() } });
        let rows = rows(&state, &Strings::new("en"));
        assert!(!rows[0].enabled);
        assert_eq!(rows[2].label, "Online News");
        assert_eq!(rows[2].button, "Edit");
        let Some(Change::Config(patch)) = &rows[2].change else {
            panic!("missing editor values")
        };
        assert_eq!(patch["internetRadioDraft"]["name"], "Online News");
        assert_eq!(
            patch["internetRadioDraft"]["url"],
            "https://radio.example/live"
        );
        assert!(matches!(
            rows[2].tools.last().unwrap().action,
            Action::InternetRadioDelete {
                service_id: u32::MAX
            }
        ));
    }

    #[test]
    fn saving_sends_the_draft_and_waits_for_the_core_to_clear_it() {
        let draft = InternetRadioDraft {
            name: "News".into(),
            url: "https://radio.example/live".into(),
            ..Default::default()
        };
        let rows = rows(
            &json!({ "config": { "internetRadioDraft": draft } }),
            &Strings::new("en"),
        );
        assert!(rows[0].enabled);
        assert_eq!(rows[0].action, Some(Action::InternetRadioSave { draft }));
        assert!(rows[0].change.is_none());
    }
}
