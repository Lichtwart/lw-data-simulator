//! Native Core v2 payloads (PRD §7): pure functions, no I/O.
//!
//! Everything that decides *how* a value looks in Cumulocity lives here so it can
//! be unit-tested against the firmware (`docs/research/firmware-core-v2.md`):
//! - [`naming`]: fragment / series / alarm type names (native DI series, D1), slave id /
//!   address and register address assignment.
//! - [`measurement`]: measurement objects and `measurementCollection` bodies (≤ 6 DALI lamps
//!   per message, batching of native fragments in firmware order).
//! - [`alarm`]: alarm bodies, the native fault catalog (DALI lamp / bus, sensors), Modbus
//!   timeout, per-slave and device-level custom alarms, fault target resolution.
//! - [`event`]: event bodies (`lw_daliGroupChange`, `lw_modbus`, `lw_alarm` "Alarm cleared").
//! - [`config`]: the demo device managed object (§6.1) and the `lw_config*` fragments
//!   (`lw_configCore`, `lw_configGroups`, `lw_configDigitalInputs`, `lw_configModbusSlaves`
//!   incl. `writeRegisters`).
//! - [`floorplan`]: library floorplan layout → `lw_floorplan` managed object (item references
//!   resolved to fragments / series, alarm types and Modbus targets).
//!
//! Conventions: every builder takes `time` (ISO-8601, the caller's "now") and puts it on every
//! measurement / alarm / event; values are JSON numbers (integral values without `.0`); series
//! with an empty firmware unit carry no `unit`.

pub mod alarm;
pub mod config;
pub mod event;
pub mod floorplan;
pub mod measurement;
pub mod naming;

use serde_json::Value;

/// `value` as a JSON number; integral values are written without a fraction (`100`, not
/// `100.0`) so integer-typed firmware fields and the plugin's `===` comparisons see what the
/// firmware would send. Non-finite values become `null` (callers reject them first).
pub fn json_number(value: f64) -> Value {
    const MAX_EXACT: f64 = 9_007_199_254_740_992.0; // 2^53
    if value.is_finite() && value.fract() == 0.0 && value.abs() < MAX_EXACT {
        // Exact: integral and within the f64-exact integer range.
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

/// `{"id": "<deviceId>"}` source reference.
fn source(device_id: &str) -> Value {
    serde_json::json!({ "id": device_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_numbers_have_no_fraction() {
        assert_eq!(json_number(100.0).to_string(), "100");
        assert_eq!(json_number(-18.0).to_string(), "-18");
        assert_eq!(json_number(-18.5).to_string(), "-18.5");
        assert_eq!(json_number(0.1).to_string(), "0.1");
        assert_eq!(json_number(f64::NAN), Value::Null);
    }
}
