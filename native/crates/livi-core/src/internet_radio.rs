// SPDX-FileCopyrightText: 2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use livi_core_proto::state::{DabStation, InternetRadioDraft, InternetStation};

pub fn validate(draft: &InternetRadioDraft) -> Result<(), String> {
    if draft.name.trim().is_empty() || draft.name.chars().count() > 128 {
        return Err("Enter a station name (up to 128 characters)".into());
    }
    if draft.description.chars().count() > 512 {
        return Err("Description must be at most 512 characters".into());
    }
    let url =
        reqwest::Url::parse(draft.url.trim()).map_err(|_| "Enter an HTTP or HTTPS stream URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || draft.url.len() > 4096
    {
        return Err("Enter an HTTP or HTTPS stream URL".into());
    }
    Ok(())
}

pub fn save(list: &mut Vec<DabStation>, draft: InternetRadioDraft) -> Result<u32, String> {
    validate(&draft)?;
    let id = if let Some(id) = draft.service_id {
        if !list
            .iter()
            .any(|s| s.service_id == id && s.internet.is_some())
        {
            return Err("Internet radio station no longer exists".into());
        }
        id
    } else {
        (1..=u32::MAX)
            .rev()
            .find(|id| !list.iter().any(|s| s.service_id == *id))
            .ok_or("Station list is full")?
    };
    let internet = Some(InternetStation {
        url: draft.url.trim().into(),
        description: draft.description.trim().into(),
    });
    if let Some(st) = list.iter_mut().find(|s| s.service_id == id) {
        st.label = draft.name.trim().into();
        st.internet = internet;
    } else {
        list.push(DabStation {
            service_id: id,
            label: draft.name.trim().into(),
            short_label: String::new(),
            ensemble: String::new(),
            channel: String::new(),
            frequency_hz: 0,
            favorite: false,
            hidden: false,
            logo: None,
            internet,
        });
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> InternetRadioDraft {
        InternetRadioDraft {
            name: "  Test Radio  ".into(),
            url: " https://radio.example/live.mp3 ".into(),
            description: "  Local news  ".into(),
            ..Default::default()
        }
    }

    #[test]
    fn streams_are_saved_and_edited_without_losing_station_preferences() {
        let mut stations = Vec::new();
        let id = save(&mut stations, draft()).unwrap();
        stations[0].favorite = true;
        stations[0].hidden = true;
        let mut edited = draft();
        edited.service_id = Some(id);
        edited.name = "Renamed Radio".into();
        edited.url = "http://radio.example/new.aac".into();
        save(&mut stations, edited).unwrap();
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0].label, "Renamed Radio");
        assert!(stations[0].favorite && stations[0].hidden);
        assert_eq!(
            stations[0].internet.as_ref().unwrap().description,
            "Local news"
        );
        assert_eq!(
            stations[0].internet.as_ref().unwrap().url,
            "http://radio.example/new.aac"
        );
        let restored: Vec<DabStation> =
            serde_json::from_str(&serde_json::to_string(&stations).unwrap()).unwrap();
        assert_eq!(restored, stations);
    }

    #[test]
    fn invalid_streams_leave_the_station_list_unchanged() {
        let mut stations = Vec::new();
        for url in [
            "file:///tmp/audio",
            "ftp://radio.example/live",
            "https://",
            "not a URL",
        ] {
            let mut d = draft();
            d.url = url.into();
            assert!(save(&mut stations, d).is_err(), "{url}");
        }
        let mut d = draft();
        d.name = "  ".into();
        assert!(save(&mut stations, d).is_err());
        let mut d = draft();
        d.service_id = Some(123);
        assert!(save(&mut stations, d).is_err());
        assert!(stations.is_empty());
    }
}
