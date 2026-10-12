//! The "generate series" helper (PRD §11.3).

use serde_json::{Value, json};

use crate::error::{AppError, AppResult};
use crate::model::{GenerateSeriesRequest, ItemKind, LibraryItem};
use crate::payload::naming::MAX_NAME_LEN;

/// Max items per call (= DALI lamps per bus).
pub const MAX_SERIES_COUNT: u32 = 64;

/// Highest native index of a kind (`None` for kinds without `index`).
fn max_index(kind: ItemKind) -> Option<u32> {
    match kind {
        ItemKind::Dali => Some(64),
        ItemKind::DaliBrightness
        | ItemKind::DaliMotion
        | ItemKind::DigitalInput
        | ItemKind::ImpulseCounter => Some(10),
        ItemKind::System | ItemKind::Modbus | ItemKind::CustomAlarm => None,
    }
}

/// Clone `req.template` `req.count` times: ids `<template id>-<n>` for native kinds (the
/// library convention, `light-1`) and `<template id>_<n>` for the others (the template id is
/// shortened so the result stays ≤ 19 chars), German / English titles `<title> <n>`. `n` runs
/// from `req.start`, and a native `index` is `n` too, so `light-5` is always lamp 5 (the
/// template's own `index` is only a placeholder). Pure.
pub fn generate(req: &GenerateSeriesRequest) -> AppResult<Vec<LibraryItem>> {
    let bad = |m: String| AppError::InvalidInput(m);
    if req.count == 0 || req.count > MAX_SERIES_COUNT {
        return Err(bad(format!("count {} (1–{MAX_SERIES_COUNT})", req.count)));
    }
    let kind = req.template.kind();
    if kind == ItemKind::System {
        return Err(bad("system items cannot be generated as a series".into()));
    }
    let template =
        serde_json::to_value(&req.template).map_err(|e| AppError::Internal(e.to_string()))?;
    let base_id = req.template.id();
    let base_index = req.template.index();
    let last_n = req.start.saturating_add(req.count - 1);
    let sep = if base_index.is_some() { '-' } else { '_' };
    let suffix_len = format!("{sep}{last_n}").len();
    let keep = MAX_NAME_LEN.saturating_sub(suffix_len);
    if keep == 0 {
        return Err(bad(format!("start {} too large", req.start)));
    }
    let short: String = base_id.chars().take(keep).collect();

    (0..req.count)
        .map(|i| {
            let n = req.start + i;
            let mut v: Value = template.clone();
            v["id"] = json!(format!("{short}{sep}{n}"));
            for lang in ["de", "en"] {
                let t = v["title"][lang].as_str().unwrap_or_default().to_owned();
                v["title"][lang] = json!(format!("{t} {n}"));
            }
            if let (Some(_), Some(max)) = (base_index, max_index(kind)) {
                let index = n;
                if index == 0 || index > max {
                    return Err(bad(format!("index {index} exceeds {max} for {kind:?}")));
                }
                v["index"] = json!(index);
            }
            serde_json::from_value(v).map_err(|e| AppError::Internal(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(template: Value, count: u32, start: u32) -> GenerateSeriesRequest {
        GenerateSeriesRequest {
            template: serde_json::from_value(template).unwrap(),
            count,
            start,
        }
    }

    #[test]
    fn numbers_ids_titles_and_indexes() {
        let out = generate(&req(
            json!({"kind": "dali", "id": "light", "index": 5,
                   "title": {"de": "Licht", "en": "Light"}, "tags": ["retail"]}),
            3,
            1,
        ))
        .unwrap();
        let ids: Vec<&str> = out.iter().map(LibraryItem::id).collect();
        assert_eq!(ids, vec!["light-1", "light-2", "light-3"]);
        assert_eq!(out[2].title().de, "Licht 3");
        assert_eq!(out[2].title().en, "Light 3");
        // `light-3` is lamp 3, whatever index the template had.
        assert_eq!(out[2].index(), Some(3));
        assert_eq!(out[0].index(), Some(1));
        let from_five = generate(&req(
            json!({"kind": "dali", "id": "light", "index": 1,
                   "title": {"de": "Licht", "en": "Light"}}),
            2,
            5,
        ))
        .unwrap();
        assert_eq!(from_five[1].id(), "light-6");
        assert_eq!(from_five[1].index(), Some(6));
        assert_eq!(out[0].tags(), ["retail".to_owned()]);
    }

    #[test]
    fn ids_are_shortened_to_19_chars() {
        let out = generate(&req(
            json!({"kind": "modbus", "id": "charging_power_kw_x", "slave": "ev",
                   "title": {"de": "Leistung", "en": "Power"}}),
            12,
            1,
        ))
        .unwrap();
        assert!(out.iter().all(|i| i.id().len() <= 19));
        // Non-native kinds keep the `_` separator.
        assert_eq!(out[11].id(), "charging_power_k_12");
    }

    #[test]
    fn rejects_out_of_range() {
        let lamp = json!({"kind": "dali", "id": "l", "index": 60, "title": {"de": "L", "en": "L"}});
        assert!(generate(&req(lamp.clone(), 64, 1)).is_ok());
        assert!(generate(&req(lamp.clone(), 6, 60)).is_err());
        assert!(generate(&req(lamp.clone(), 5, 60)).is_ok());
        assert!(generate(&req(lamp, 0, 1)).is_err());
        let sys =
            json!({"kind": "system", "id": "r", "system": "rssi", "title": {"de": "R", "en": "R"}});
        assert!(generate(&req(sys, 2, 1)).is_err());
    }
}
