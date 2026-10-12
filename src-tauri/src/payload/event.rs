//! Event bodies (firmware-core-v2.md §3).

use serde_json::{Map, Value, json};

use crate::model::GroupMode;

/// `lw_daliGroupChange` reason for a cloud operation (the firmware also sends `digital-input-NN`).
pub const REASON_CLOUD_OPERATION: &str = "cloud-operation";

/// Firmware spelling of a group mode in `lw_daliGroupChange`: `On` / `Off` / `Auto` (§3.3).
pub fn group_mode_label(mode: GroupMode) -> &'static str {
    match mode {
        GroupMode::On => "On",
        GroupMode::Off => "Off",
        GroupMode::Auto => "Auto",
    }
}

/// `POST /event/event` body. `extra` fragments are merged in (e.g. multi-effect items).
pub fn build_event(
    device_id: &str,
    time: &str,
    event_type: &str,
    text: &str,
    extra: Option<Map<String, Value>>,
) -> Value {
    let mut body = Map::new();
    if let Some(extra) = extra {
        body.extend(extra);
    }
    body.insert("source".into(), super::source(device_id));
    body.insert("type".into(), json!(event_type));
    body.insert("text".into(), json!(text));
    body.insert("time".into(), json!(time));
    Value::Object(body)
}

/// `lw_daliGroupChange` (§3.3): text `Groupmode changed: <reason> - Group: <N> - From <Prev> to
/// <New>` and `groupModeChange {id (1-based), previousMode, newMode, reason}` with capitalised
/// modes. The firmware only sends it when the mode really changed; so should the caller.
pub fn build_group_change(
    device_id: &str,
    time: &str,
    group_id: u32,
    previous: GroupMode,
    new: GroupMode,
    reason: &str,
) -> Value {
    let (prev, next) = (group_mode_label(previous), group_mode_label(new));
    let mut extra = Map::new();
    extra.insert(
        "groupModeChange".into(),
        json!({ "id": group_id, "previousMode": prev, "newMode": next, "reason": reason }),
    );
    build_event(
        device_id,
        time,
        "lw_daliGroupChange",
        &format!("Groupmode changed: {reason} - Group: {group_id} - From {prev} to {next}"),
        Some(extra),
    )
}

/// `lw_alarm` "Alarm cleared: <type>" (§3.1), sent when a fault recovers.
pub fn build_alarm_cleared(device_id: &str, time: &str, alarm_type: &str) -> Value {
    build_event(
        device_id,
        time,
        "lw_alarm",
        &format!("Alarm cleared: {alarm_type}"),
        None,
    )
}

/// `lw_modbus` at the transition into the timeout state (§3.2):
/// `Modbus slave <id:02>-<addr:03> (<name>): Timeout alarm: Timeout - slave not responding`
/// plus the register suffix when the slave has registers: ` - register <first>` when
/// `first == last`, else ` - registers <first>-<last>` (`MBM_modbus_master.c:756-772`).
/// `registers` is the first contiguous read bundle in **words** (a `float32` is 2 words), so
/// one 32-bit register gives `(a, a + 1)`.
pub fn build_modbus_timeout_event(
    device_id: &str,
    time: &str,
    slave_id: u32,
    slave_address: u32,
    slave_name: &str,
    registers: Option<(u32, u32)>,
) -> Value {
    let mut text = format!(
        "Modbus slave {slave_id:02}-{slave_address:03} ({slave_name}): Timeout alarm: \
         Timeout - slave not responding"
    );
    match registers {
        Some((first, last)) if first == last => text.push_str(&format!(" - register {first}")),
        Some((first, last)) => text.push_str(&format!(" - registers {first}-{last}")),
        None => {}
    }
    build_event(device_id, time, "lw_modbus", &text, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "2026-10-12T12:00:00.000Z";

    /// JSON equivalent of the firmware `3200` message (§3.3).
    #[test]
    fn group_change_matches_firmware() {
        assert_eq!(
            build_group_change(
                "7",
                T,
                4,
                GroupMode::Auto,
                GroupMode::On,
                REASON_CLOUD_OPERATION
            ),
            json!({
                "source": {"id": "7"}, "type": "lw_daliGroupChange", "time": T,
                "text": "Groupmode changed: cloud-operation - Group: 4 - From Auto to On",
                "groupModeChange": {"id": 4, "previousMode": "Auto", "newMode": "On",
                                    "reason": "cloud-operation"}
            })
        );
    }

    #[test]
    fn alarm_cleared_text() {
        let e = build_alarm_cleared("7", T, "lw_lampFailure_1_05_shortCircuit");
        assert_eq!(e["type"], "lw_alarm");
        assert_eq!(e["text"], "Alarm cleared: lw_lampFailure_1_05_shortCircuit");
        assert_eq!(e["time"], T);
    }

    #[test]
    fn modbus_timeout_text() {
        let e = build_modbus_timeout_event("7", T, 20, 240, "Kuehlung", Some((30001, 30004)));
        assert_eq!(e["type"], "lw_modbus");
        assert_eq!(
            e["text"],
            "Modbus slave 20-240 (Kuehlung): Timeout alarm: Timeout - slave not responding \
             - registers 30001-30004"
        );
        let e = build_modbus_timeout_event("7", T, 1, 5, "Klima", Some((40001, 40001)));
        assert_eq!(
            e["text"],
            "Modbus slave 01-005 (Klima): Timeout alarm: Timeout - slave not responding \
             - register 40001"
        );
        let e = build_modbus_timeout_event("7", T, 1, 5, "Klima", None);
        assert_eq!(
            e["text"],
            "Modbus slave 01-005 (Klima): Timeout alarm: Timeout - slave not responding"
        );
    }

    #[test]
    fn extra_fragments_cannot_override_core_fields() {
        let mut extra = Map::new();
        extra.insert("type".into(), json!("hijack"));
        extra.insert("lw_acMode".into(), json!({"value": 2}));
        let e = build_event("7", T, "lw_demo_acMode", "Kühlen", Some(extra));
        assert_eq!(e["type"], "lw_demo_acMode");
        assert_eq!(e["lw_acMode"]["value"], 2);
    }
}
