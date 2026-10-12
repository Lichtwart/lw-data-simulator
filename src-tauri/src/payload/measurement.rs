//! Measurement bodies (firmware-core-v2.md §1, "Posting via REST").
//!
//! One firmware `201` line = one measurement object. A DALI lamp line carries four series under
//! `LW Dali Control Gear 1-NN`; every other native line carries one series. Objects are batched
//! into `measurementCollection` bodies in firmware order: sensors + system, digital inputs, DALI
//! lamps (≤ 6 per collection, `CLOUD_SEND_LIMIT`), Modbus, anything else.

use serde_json::{Map, Value, json};

use super::json_number;
use super::naming::{
    DALI_BRIGHTNESS_TYPE, DALI_MEASUREMENT_TYPE, DALI_MOTION_TYPE, DALI_SERIES, DALI_VALUE_SERIES,
    DI_MEASUREMENT_TYPE, MODBUS_MEASUREMENT_TYPE, RSSI_TYPE, TEMPERATURE_TYPE, dali_fragment,
};
use crate::error::{AppError, AppResult};
use crate::model::MeasurementNaming;

/// Max DALI lamps per message (`CLOUD_SEND_LIMIT 6`).
pub const MAX_DALI_LAMPS_PER_MESSAGE: usize = 6;
/// Content type of a collection body (`POST /measurement/measurements`).
pub const COLLECTION_CONTENT_TYPE: &str =
    "application/vnd.com.nsn.cumulocity.measurementcollection+json";

/// One series value to send: resolved naming + value + unit (`None` = omit `unit`).
#[derive(Debug, Clone, PartialEq)]
pub struct MeasurementValue {
    pub measurement_type: String,
    pub fragment: String,
    pub series: String,
    pub value: f64,
    pub unit: Option<String>,
}

/// The non-brightness series of a DALI lamp. Status values: 1 = OK, 0 = failure (§1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DaliLampStatus {
    /// `Converter Status` (control gear OK).
    pub converter_ok: bool,
    /// `Lamp Status`.
    pub lamp_ok: bool,
    /// `Converter Runtime`, integer hours.
    pub runtime_hours: u32,
}

impl Default for DaliLampStatus {
    fn default() -> Self {
        Self {
            converter_ok: true,
            lamp_ok: true,
            runtime_hours: 0,
        }
    }
}

/// Grid of a digital-input impulse counter: the firmware sends `initCount + k·increment`
/// (`%u`). Pass it to [`item_values_with_grid`] / [`quantize`]; without it a counter is floored
/// to an integer (`initCount` 0, `increment` 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterGrid {
    pub init_count: u32,
    pub increment: u32,
}

impl Default for CounterGrid {
    fn default() -> Self {
        Self {
            init_count: 0,
            increment: 1,
        }
    }
}

/// Make `value` look like what the firmware can send for this series (it only emits `%u`
/// integers for most series and `%.1f` for the board temperature):
/// - DALI `Brightness` rounded and clamped to 0-100, `Converter Runtime` rounded and ≥ 0,
///   statuses 0 / 1;
/// - `Lux<N>` rounded and ≥ 0; motion `State<N>` and DI `Value<NN>` clamped to 0 / 1;
/// - DI `ImpulseCounter<NN>` floored to `initCount + k·increment` (never below `initCount`);
/// - `rssi` rounded; board temperature rounded to 0.1;
/// - Modbus and unknown series are left raw (the register datatype decides).
///
/// Non-finite values are returned unchanged (the body builder rejects them).
pub fn quantize(
    measurement_type: &str,
    series: &str,
    value: f64,
    grid: Option<CounterGrid>,
) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let bit = |v: f64| if v >= 0.5 { 1.0 } else { 0.0 };
    match measurement_type {
        DALI_MEASUREMENT_TYPE => match series {
            "Brightness" => value.round().clamp(0.0, 100.0),
            "Converter Runtime" => value.round().max(0.0),
            _ => bit(value),
        },
        DALI_BRIGHTNESS_TYPE => value.round().max(0.0),
        DALI_MOTION_TYPE => bit(value),
        RSSI_TYPE => value.round(),
        TEMPERATURE_TYPE => (value * 10.0).round() / 10.0,
        DI_MEASUREMENT_TYPE if series.starts_with("ImpulseCounter") => {
            let g = grid.unwrap_or_default();
            let init = f64::from(g.init_count);
            if value <= init {
                return init;
            }
            let step = f64::from(g.increment.max(1));
            let k = ((value - init) / step).floor();
            (init + k * step).min(f64::from(u32::MAX))
        }
        DI_MEASUREMENT_TYPE => bit(value),
        _ => value,
    }
}

/// The values an item sends for `value`, quantized to what the firmware emits (see
/// [`quantize`]; DI counters use the default grid, use [`item_values_with_grid`] for
/// `initCount` / `increment`). Single-series items give one value; a DALI lamp gives all four
/// series in firmware order (`value` → `Brightness`, the rest from `dali` or OK / 0 h).
pub fn item_values(
    naming: &MeasurementNaming,
    value: f64,
    dali: Option<DaliLampStatus>,
) -> Vec<MeasurementValue> {
    item_values_with_grid(naming, value, dali, None)
}

/// [`item_values`] with the DI impulse counter grid (`initCount` / `increment` of the device's
/// `lw_configDigitalInputs` entry).
pub fn item_values_with_grid(
    naming: &MeasurementNaming,
    value: f64,
    dali: Option<DaliLampStatus>,
    grid: Option<CounterGrid>,
) -> Vec<MeasurementValue> {
    let make = |series: &str, unit: Option<&str>, value: f64| MeasurementValue {
        measurement_type: naming.measurement_type.clone(),
        fragment: naming.fragment.clone(),
        series: series.to_owned(),
        value: quantize(&naming.measurement_type, series, value, grid),
        unit: unit.map(str::to_owned),
    };
    if naming.measurement_type == DALI_MEASUREMENT_TYPE {
        let s = dali.unwrap_or_default();
        return naming
            .series
            .iter()
            .map(|n| {
                let v = match n.name.as_str() {
                    DALI_VALUE_SERIES => value,
                    "Converter Status" => f64::from(u8::from(s.converter_ok)),
                    "Lamp Status" => f64::from(u8::from(s.lamp_ok)),
                    "Converter Runtime" => f64::from(s.runtime_hours),
                    _ => value,
                };
                make(&n.name, n.unit.as_deref(), v)
            })
            .collect();
    }
    naming
        .series
        .iter()
        .map(|n| make(&n.name, n.unit.as_deref(), value))
        .collect()
}

/// The four values of DALI lamp `lamp` (1–64) without a resolved naming.
pub fn dali_lamp_values(
    lamp: u32,
    brightness: f64,
    status: DaliLampStatus,
) -> Vec<MeasurementValue> {
    let naming = MeasurementNaming {
        measurement_type: DALI_MEASUREMENT_TYPE.to_owned(),
        fragment: dali_fragment(lamp),
        series: DALI_SERIES
            .iter()
            .map(|(n, u)| crate::model::SeriesNaming {
                name: (*n).to_owned(),
                unit: u.map(str::to_owned),
            })
            .collect(),
    };
    item_values(&naming, brightness, Some(status))
}

/// One measurement object (single `201` line) for `values` that share type + fragment.
fn measurement_object(device_id: &str, time: &str, values: &[&MeasurementValue]) -> Value {
    let first = values[0];
    let mut fragment = Map::new();
    for v in values {
        let mut series = Map::new();
        series.insert("value".into(), json_number(v.value));
        if let Some(unit) = v.unit.as_deref().filter(|u| !u.is_empty()) {
            series.insert("unit".into(), json!(unit));
        }
        fragment.insert(v.series.clone(), Value::Object(series));
    }
    let mut body = Map::new();
    body.insert("source".into(), super::source(device_id));
    body.insert("type".into(), json!(first.measurement_type));
    body.insert("time".into(), json!(time));
    body.insert(first.fragment.clone(), Value::Object(fragment));
    Value::Object(body)
}

/// Batch bucket in firmware send order.
fn bucket(measurement_type: &str) -> usize {
    match measurement_type {
        DALI_BRIGHTNESS_TYPE | DALI_MOTION_TYPE | RSSI_TYPE | TEMPERATURE_TYPE => 0,
        DI_MEASUREMENT_TYPE => 1,
        DALI_MEASUREMENT_TYPE => 2,
        MODBUS_MEASUREMENT_TYPE => 3,
        _ => 4,
    }
}

/// Build the `measurementCollection` bodies (`{"measurements": [...]}`) for `values`, all with
/// `time`. DALI series of the same lamp are merged into one object (later duplicates of a series
/// win); every other value is its own object. Fails on a non-finite value (JSON numbers only).
pub fn build_measurement_bodies(
    device_id: &str,
    time: &str,
    values: &[MeasurementValue],
) -> AppResult<Vec<Value>> {
    if let Some(bad) = values.iter().find(|v| !v.value.is_finite()) {
        return Err(AppError::InvalidInput(format!(
            "{}.{}: value {} is not a finite number",
            bad.fragment, bad.series, bad.value
        )));
    }
    // Group into objects: DALI per fragment (first-seen order), everything else one per value.
    let mut objects: Vec<Vec<&MeasurementValue>> = Vec::new();
    for v in values {
        if v.measurement_type == DALI_MEASUREMENT_TYPE
            && let Some(obj) = objects.iter_mut().find(|o| {
                o[0].measurement_type == v.measurement_type && o[0].fragment == v.fragment
            })
        {
            obj.retain(|x| x.series != v.series);
            obj.push(v);
            continue;
        }
        objects.push(vec![v]);
    }

    let mut buckets: [Vec<Value>; 5] = Default::default();
    for obj in &objects {
        buckets[bucket(&obj[0].measurement_type)].push(measurement_object(device_id, time, obj));
    }
    let mut bodies = Vec::new();
    for (i, list) in buckets.into_iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        if i == 2 {
            for chunk in list.chunks(MAX_DALI_LAMPS_PER_MESSAGE) {
                bodies.push(json!({ "measurements": chunk }));
            }
        } else {
            bodies.push(json!({ "measurements": list }));
        }
    }
    Ok(bodies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SeriesNaming;

    const T: &str = "2026-10-12T12:00:00.000Z";

    fn single(
        t: &str,
        fragment: &str,
        series: &str,
        value: f64,
        unit: Option<&str>,
    ) -> MeasurementValue {
        MeasurementValue {
            measurement_type: t.into(),
            fragment: fragment.into(),
            series: series.into(),
            value,
            unit: unit.map(str::to_owned),
        }
    }

    /// DALI line (§1.2): one object, four series, empty units omitted, `time` set.
    #[test]
    fn dali_lamp_is_one_object_with_four_series() {
        let status = DaliLampStatus {
            converter_ok: true,
            lamp_ok: false,
            runtime_hours: 1234,
        };
        let bodies = build_measurement_bodies("9", T, &dali_lamp_values(3, 80.0, status)).unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0],
            json!({"measurements": [{
                "source": {"id": "9"}, "type": "daliControlGear", "time": T,
                "LW Dali Control Gear 1-03": {
                    "Converter Status": {"value": 1},
                    "Lamp Status": {"value": 0},
                    "Converter Runtime": {"value": 1234, "unit": "hr"},
                    "Brightness": {"value": 80, "unit": "%"}
                }
            }]})
        );
    }

    #[test]
    fn at_most_six_lamps_per_message() {
        let values: Vec<MeasurementValue> = (1..=13)
            .flat_map(|lamp| dali_lamp_values(lamp, 50.0, DaliLampStatus::default()))
            .collect();
        let bodies = build_measurement_bodies("9", T, &values).unwrap();
        let sizes: Vec<usize> = bodies
            .iter()
            .map(|b| b["measurements"].as_array().unwrap().len())
            .collect();
        assert_eq!(sizes, vec![6, 6, 1]);
    }

    #[test]
    fn native_names_units_and_batching_order() {
        let values = vec![
            single(
                "modbus",
                "LW_Modbus_Slave_20_240",
                "freezer_temp",
                -18.2,
                Some("°C"),
            ),
            single("digitalInput", "LW Digital Input", "Value01", 1.0, None),
            single(
                "digitalInput",
                "LW Digital Input",
                "ImpulseCounter03",
                1010.0,
                Some("m3"),
            ),
            single(
                "c8y_SignalStrength",
                "c8y_SignalStrength",
                "rssi",
                -67.0,
                Some("dBm"),
            ),
            single(
                "c8y_TemperatureMeasurement",
                "c8y_TemperatureMeasurement",
                "T",
                38.5,
                Some("C"),
            ),
            single(
                "daliBrightness",
                "LW Dali Brightness",
                "Lux1",
                450.0,
                Some("Lux"),
            ),
            single("daliMotion", "LW Motion", "State1", 1.0, None),
        ];
        let bodies = build_measurement_bodies("9", T, &values).unwrap();
        assert_eq!(bodies.len(), 3);
        let first = bodies[0]["measurements"].as_array().unwrap();
        assert_eq!(first.len(), 4);
        assert_eq!(
            first[0]["c8y_SignalStrength"]["rssi"],
            json!({"value": -67, "unit": "dBm"})
        );
        assert_eq!(
            first[1]["c8y_TemperatureMeasurement"]["T"],
            json!({"value": 38.5, "unit": "C"})
        );
        assert_eq!(first[2]["LW Dali Brightness"]["Lux1"]["unit"], "Lux");
        assert_eq!(first[3]["LW Motion"]["State1"], json!({"value": 1}));
        // DI: one object per series, like one `201` line each.
        let di = bodies[1]["measurements"].as_array().unwrap();
        assert_eq!(di.len(), 2);
        assert_eq!(di[0]["type"], "digitalInput");
        assert_eq!(di[0]["LW Digital Input"]["Value01"], json!({"value": 1}));
        assert_eq!(di[1]["LW Digital Input"]["ImpulseCounter03"]["unit"], "m3");
        let mb = &bodies[2]["measurements"][0];
        assert_eq!(mb["type"], "modbus");
        assert_eq!(
            mb["LW_Modbus_Slave_20_240"]["freezer_temp"]["value"],
            json!(-18.2)
        );
        for b in &bodies {
            for m in b["measurements"].as_array().unwrap() {
                assert_eq!(m["time"], T);
                assert_eq!(m["source"]["id"], "9");
            }
        }
    }

    #[test]
    fn rejects_non_finite_values() {
        let v = [single("modbus", "F", "s", f64::NAN, None)];
        assert!(build_measurement_bodies("9", T, &v).is_err());
        assert!(build_measurement_bodies("9", T, &[]).unwrap().is_empty());
    }

    #[test]
    fn item_values_follow_naming() {
        let naming = MeasurementNaming {
            measurement_type: "digitalInput".into(),
            fragment: "LW Digital Input".into(),
            series: vec![SeriesNaming {
                name: "Value02".into(),
                unit: None,
            }],
        };
        let v = item_values(&naming, 1.0, None);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].series.as_str(), v[0].value), ("Value02", 1.0));
        let lamp = dali_lamp_values(1, 42.0, DaliLampStatus::default());
        assert_eq!(lamp.len(), 4);
        assert_eq!(lamp[3].series, "Brightness");
        assert_eq!(lamp[3].value, 42.0);
        assert_eq!(lamp[0].value, 1.0);
    }

    #[test]
    fn quantize_follows_firmware_types() {
        let q = |t: &str, s: &str, v: f64| quantize(t, s, v, None);
        // DALI
        assert_eq!(q("daliControlGear", "Brightness", 57.349), 57.0);
        assert_eq!(q("daliControlGear", "Brightness", 140.0), 100.0);
        assert_eq!(q("daliControlGear", "Brightness", -3.0), 0.0);
        assert_eq!(q("daliControlGear", "Converter Runtime", 12.6), 13.0);
        assert_eq!(q("daliControlGear", "Lamp Status", 0.7), 1.0);
        // Sensors
        assert_eq!(q("daliBrightness", "Lux1", 450.4), 450.0);
        assert_eq!(q("daliBrightness", "Lux1", -5.0), 0.0);
        assert_eq!(q("daliMotion", "State1", 0.4), 0.0);
        assert_eq!(q("daliMotion", "State1", 3.0), 1.0);
        // System
        assert_eq!(q("c8y_SignalStrength", "rssi", -67.4), -67.0);
        assert_eq!(q("c8y_TemperatureMeasurement", "T", 38.449), 38.4);
        // DI value: only 0 / 1
        assert_eq!(q("digitalInput", "Value01", 0.3), 0.0);
        assert_eq!(q("digitalInput", "Value01", 7.0), 1.0);
        // Modbus stays raw
        assert_eq!(q("modbus", "freezer_temp", -18.237), -18.237);
        assert!(q("modbus", "x", f64::NAN).is_nan());
    }

    #[test]
    fn counter_is_floored_to_the_init_increment_grid() {
        let g = Some(CounterGrid {
            init_count: 1000,
            increment: 10,
        });
        let q = |v| quantize("digitalInput", "ImpulseCounter03", v, g);
        assert_eq!(q(1000.1), 1000.0);
        assert_eq!(q(1019.9), 1010.0);
        assert_eq!(q(1020.0), 1020.0);
        assert_eq!(q(5.0), 1000.0);
        // No grid: plain integer floor.
        assert_eq!(
            quantize("digitalInput", "ImpulseCounter03", 1000.7, None),
            1000.0
        );
    }

    #[test]
    fn item_values_are_quantized() {
        let lamp = dali_lamp_values(1, 57.349, DaliLampStatus::default());
        assert_eq!(lamp[3].value, 57.0);
        let naming = MeasurementNaming {
            measurement_type: "digitalInput".into(),
            fragment: "LW Digital Input".into(),
            series: vec![SeriesNaming {
                name: "ImpulseCounter03".into(),
                unit: Some("m3".into()),
            }],
        };
        let v = item_values_with_grid(
            &naming,
            1006.0,
            None,
            Some(CounterGrid {
                init_count: 1000,
                increment: 5,
            }),
        );
        assert_eq!(v[0].value, 1005.0);
        assert_eq!(item_values(&naming, 1000.1, None)[0].value, 1000.0);
    }
}
