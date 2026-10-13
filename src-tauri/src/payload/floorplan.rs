//! Library floorplan layout → `lw_floorplan` managed object (PRD §12,
//! docs/research/floorplan-plugin.md §1, §3, §7).
//!
//! Item references are resolved here: `measurement {fragment, series}` from the item naming
//! (title / item id, D10), `alarmTypes` from alarm references (written literally, the plugin never suggests
//! custom types), Modbus control targets from the slave / register assignment.

use serde_json::{Map, Value, json};

use super::alarm::{
    brightness_alarms, dali_lamp_alarms, device_custom_alarm, motion_alarms, slave_custom_alarms,
    slave_timeout_alarm,
};
use super::json_number;
use super::naming::{self, DALI_VALUE_SERIES, item_naming_with_di};
use crate::error::{AppError, AppResult};
use crate::model::{
    AlarmRef, DaliGroup, DigitalInputEntry, FloorplanLayout, FloorplanWidget, LibraryItem,
    ModbusValueType, SensorDisplay, SlaveAssignment, SlaveDefinition, WidgetControl, WidgetLevel,
};

/// Managed-object `type` of a floorplan.
pub const FLOORPLAN_TYPE: &str = "lw_floorplan";

/// The resolved device setup a floorplan is applied to.
#[derive(Debug, Clone, Copy)]
pub struct FloorplanContext<'a> {
    /// The demo's items (library selection plus custom items).
    pub items: &'a [&'a LibraryItem],
    /// Slaves with their persisted assignments (ids, addresses, register addresses).
    pub slaves: &'a [(&'a SlaveDefinition, &'a SlaveAssignment)],
    /// DALI groups of the device (group names for DALI control titles).
    pub groups: &'a [DaliGroup],
    /// Digital inputs of the device (titles for DI widgets and measurement fragments).
    pub digital_inputs: &'a [DigitalInputEntry],
}

impl<'a> FloorplanContext<'a> {
    fn item(&self, id: &str) -> AppResult<&'a LibraryItem> {
        self.items
            .iter()
            .find(|i| i.id() == id)
            .copied()
            .ok_or_else(|| AppError::NotFound(format!("item {id}")))
    }

    fn slave(&self, key: &str) -> AppResult<(&'a SlaveDefinition, &'a SlaveAssignment)> {
        self.slaves
            .iter()
            .find(|(d, _)| d.key == key)
            .copied()
            .ok_or_else(|| AppError::NotFound(format!("slave {key}")))
    }
}

/// All alarm types that belong to an item (see [`AlarmRef::Item`]): the native ones of its kind,
/// the slave's alarms for a Modbus item, and the alarms its `effects` raise (so a digital input
/// with a leak alarm effect is watched by its own widget).
pub fn item_alarm_types(item: &LibraryItem, ctx: &FloorplanContext<'_>) -> AppResult<Vec<String>> {
    let mut out = native_alarm_types(item, ctx)?;
    for e in item.effects() {
        if let crate::model::ItemEffect::Alarm { alarm_type, .. } = e
            && !out.contains(alarm_type)
        {
            out.push(alarm_type.clone());
        }
    }
    Ok(out)
}

fn native_alarm_types(item: &LibraryItem, ctx: &FloorplanContext<'_>) -> AppResult<Vec<String>> {
    let types = |specs: Vec<super::alarm::AlarmSpec>| specs.into_iter().map(|s| s.alarm_type);
    Ok(match item {
        LibraryItem::Dali { index, .. } => types(dali_lamp_alarms(*index)).collect(),
        LibraryItem::DaliBrightness { index, .. } => types(brightness_alarms(*index)).collect(),
        LibraryItem::DaliMotion { index, .. } => types(motion_alarms(*index)).collect(),
        LibraryItem::Modbus { slave, .. } => {
            let (def, a) = ctx.slave(slave)?;
            std::iter::once(slave_timeout_alarm(def, a).alarm_type)
                .chain(types(slave_custom_alarms(def, a)))
                .collect()
        }
        LibraryItem::CustomAlarm { .. } => device_custom_alarm(item)
            .map(|s| s.alarm_type)
            .into_iter()
            .collect(),
        LibraryItem::DigitalInput { .. }
        | LibraryItem::ImpulseCounter { .. }
        | LibraryItem::System { .. } => Vec::new(),
    })
}

fn resolve_alarm_ref(r: &AlarmRef, ctx: &FloorplanContext<'_>) -> AppResult<Vec<String>> {
    match r {
        AlarmRef::Item { item_id } => item_alarm_types(ctx.item(item_id)?, ctx),
        AlarmRef::SlaveTimeout { slave_key } => {
            let (_, a) = ctx.slave(slave_key)?;
            Ok(vec![naming::slave_timeout_alarm_type(a.id)])
        }
        AlarmRef::SlaveAlarm {
            slave_key,
            alarm_name,
        } => {
            let (def, a) = ctx.slave(slave_key)?;
            if !def.alarms.iter().any(|x| x.name == *alarm_name) {
                return Err(AppError::NotFound(format!(
                    "alarm {alarm_name} of slave {slave_key}"
                )));
            }
            Ok(vec![naming::slave_alarm_type(a.id, alarm_name)])
        }
        AlarmRef::Type { alarm_type } => Ok(vec![alarm_type.clone()]),
    }
}

/// Literal alarm types for `refs`, deduplicated in order.
fn alarm_types<'r>(
    refs: impl IntoIterator<Item = &'r AlarmRef>,
    ctx: &FloorplanContext<'_>,
) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for r in refs {
        for t in resolve_alarm_ref(r, ctx)? {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    Ok(out)
}

fn level(l: WidgetLevel) -> &'static str {
    match l {
        WidgetLevel::Good => "good",
        WidgetLevel::Warning => "warning",
        WidgetLevel::Alarm => "alarm",
    }
}

fn display_json(d: &SensorDisplay) -> Value {
    match d {
        SensorDisplay::Number {} => json!({ "kind": "number" }),
        SensorDisplay::Boolean {
            on_value,
            off_value,
            on_label,
            off_label,
            on_level,
            off_level,
            state_icons,
        } => {
            let mut o = Map::new();
            o.insert("kind".into(), json!("boolean"));
            o.insert("onValue".into(), json_number(*on_value));
            o.insert("offValue".into(), json_number(*off_value));
            o.insert("onLabel".into(), json!(on_label));
            o.insert("offLabel".into(), json!(off_label));
            if let Some(l) = on_level {
                o.insert("onLevel".into(), json!(level(*l)));
            }
            if let Some(l) = off_level {
                o.insert("offLevel".into(), json!(level(*l)));
            }
            o.insert(
                "stateIcons".into(),
                state_icons
                    .as_ref()
                    .map_or(Value::Null, |s| json!({ "on": s.on, "off": s.off })),
            );
            Value::Object(o)
        }
    }
}

/// `{fragment, series}` + series unit of an item's measurement.
fn measurement_ref(
    item: &LibraryItem,
    series: Option<&str>,
    ctx: &FloorplanContext<'_>,
) -> AppResult<(Value, Option<String>)> {
    let slave = match item.slave_key() {
        Some(key) => Some(ctx.slave(key)?.1),
        None => None,
    };
    // A DI item follows the device's digital input entry (title, counter unit, D6 / D10).
    let di = item
        .index()
        .and_then(|i| ctx.digital_inputs.iter().find(|e| e.index == i))
        .filter(|_| {
            matches!(
                item,
                LibraryItem::DigitalInput { .. } | LibraryItem::ImpulseCounter { .. }
            )
        });
    let naming = item_naming_with_di(item, slave, di)
        .ok_or_else(|| AppError::InvalidInput(format!("item {} has no measurement", item.id())))?;
    // The library names a DALI lamp's series (`Brightness`, `Lamp Status`, …); on the wire they
    // are the item id and `<id> <name>` (D10). Other items have exactly one series.
    let wanted = if naming.series.len() > 1 {
        naming::dali_series_name(item.id(), series.unwrap_or(DALI_VALUE_SERIES))
    } else {
        series.unwrap_or(naming.series[0].name.as_str()).to_owned()
    };
    let s = naming
        .series
        .iter()
        .find(|s| s.name == wanted)
        .ok_or_else(|| {
            AppError::InvalidInput(format!(
                "item {}: no series {}",
                item.id(),
                series.unwrap_or(&wanted)
            ))
        })?;
    Ok((
        json!({ "fragment": naming.fragment, "series": s.name }),
        s.unit.clone(),
    ))
}

/// Default sensor title: the German item title; for a DI the device's digital input title when
/// it has one (the same text as the measurement fragment, D10), and `Eingang <N>` when neither
/// has a title (the widget texts are German).
fn default_title(item: &LibraryItem, ctx: &FloorplanContext<'_>) -> String {
    match item {
        LibraryItem::DigitalInput { index, .. } | LibraryItem::ImpulseCounter { index, .. } => ctx
            .digital_inputs
            .iter()
            .find(|e| e.index == *index && !e.title.trim().is_empty())
            .map(|e| e.title.clone())
            .or_else(|| Some(item.title().de.clone()).filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| format!("Eingang {index}")),
        _ => item.title().de.clone(),
    }
}

fn widget_json(w: &FloorplanWidget, ctx: &FloorplanContext<'_>) -> AppResult<Value> {
    let (x, y) = w.position();
    let mut o = Map::new();
    o.insert("id".into(), json!(w.id()));
    match w {
        FloorplanWidget::Sensor {
            locked,
            icon,
            title,
            measurement,
            unit,
            decimals,
            display,
            ranges,
            auto_alarms,
            alarms,
            stale_after_min,
            ..
        } => {
            let item = ctx.item(&measurement.item_id)?;
            let (m, series_unit) = measurement_ref(item, measurement.series.as_deref(), ctx)?;
            let auto = AlarmRef::Item {
                item_id: measurement.item_id.clone(),
            };
            let refs = auto_alarms.then_some(&auto).into_iter().chain(alarms);
            o.insert("kind".into(), json!("sensor"));
            o.insert("x".into(), json_number(x));
            o.insert("y".into(), json_number(y));
            o.insert("locked".into(), json!(locked));
            o.insert("icon".into(), json!(icon));
            o.insert(
                "title".into(),
                json!(title.clone().unwrap_or_else(|| default_title(item, ctx))),
            );
            o.insert("measurement".into(), m);
            if let Some(u) = unit.clone().or(series_unit) {
                o.insert("unit".into(), json!(u));
            }
            o.insert("decimals".into(), json!(decimals.unwrap_or(1)));
            if let Some(d) = display {
                o.insert("display".into(), display_json(d));
            }
            let ranges: Vec<Value> = ranges
                .iter()
                .map(|r| {
                    json!({
                        "min": r.min.map_or(Value::Null, json_number),
                        "max": r.max.map_or(Value::Null, json_number),
                        "level": level(r.level),
                    })
                })
                .collect();
            o.insert("ranges".into(), Value::Array(ranges));
            o.insert("alarmTypes".into(), json!(alarm_types(refs, ctx)?));
            o.insert("staleAfterMin".into(), json!(stale_after_min));
        }
        FloorplanWidget::Alarm {
            locked,
            icon,
            text,
            note,
            alarms,
            ..
        } => {
            o.insert("kind".into(), json!("alarm"));
            o.insert("x".into(), json_number(x));
            o.insert("y".into(), json_number(y));
            o.insert("locked".into(), json!(locked));
            o.insert("icon".into(), json!(icon));
            o.insert("text".into(), json!(text));
            if let Some(n) = note {
                o.insert("note".into(), json!(n));
            }
            o.insert("alarmTypes".into(), json!(alarm_types(alarms, ctx)?));
        }
        FloorplanWidget::Control {
            locked,
            icon,
            title,
            confirm,
            control,
            ..
        } => {
            let (control, default_title) = control_json(control, ctx)?;
            o.insert("kind".into(), json!("control"));
            o.insert("x".into(), json_number(x));
            o.insert("y".into(), json_number(y));
            o.insert("locked".into(), json!(locked));
            o.insert("icon".into(), json!(icon));
            o.insert(
                "title".into(),
                json!(title.clone().unwrap_or(default_title)),
            );
            o.insert("confirm".into(), json!(confirm));
            o.insert("control".into(), control);
        }
    }
    Ok(Value::Object(o))
}

/// Plugin `control` object + default widget title.
fn control_json(c: &WidgetControl, ctx: &FloorplanContext<'_>) -> AppResult<(Value, String)> {
    match c {
        WidgetControl::Dali { group, modes } => {
            // Groups are edited on the device (D6): a missing group fails the apply like every
            // other unresolved reference instead of getting a made-up title.
            let title = ctx
                .groups
                .iter()
                .find(|g| g.id == *group)
                .map(|g| g.name.clone())
                .ok_or_else(|| AppError::NotFound(format!("group {group}")))?;
            Ok((
                json!({ "type": "dali", "group": group, "modes": modes }),
                title,
            ))
        }
        WidgetControl::Modbus {
            item_id,
            readback,
            value_type,
            on_label,
            off_label,
            invert,
            min,
            max,
            step,
            unit,
            options,
        } => {
            let item = ctx.item(item_id)?;
            let not_writable = || AppError::InvalidInput(format!("item {item_id} is not writable"));
            let LibraryItem::Modbus {
                slave,
                writable: true,
                ..
            } = item
            else {
                return Err(not_writable());
            };
            let register_type = item.register_type().ok_or_else(not_writable)?;
            let write_type = register_type
                .write_operation_type()
                .ok_or_else(not_writable)?;
            let (_, a) = ctx.slave(slave)?;
            let reg = a
                .registers
                .iter()
                .find(|r| r.item_id == *item_id && r.register_type == register_type)
                .ok_or_else(|| {
                    AppError::InvalidInput(format!("item {item_id} has no register address"))
                })?;
            let mut o = Map::new();
            o.insert("type".into(), json!("modbus"));
            o.insert(
                "target".into(),
                json!({
                    "source": "saved",
                    "ref": format!("{}:{}", a.address, reg.address),
                    "slaveAddress": a.address,
                    "registerAddress": reg.address,
                    "type": write_type,
                }),
            );
            let rb = if *readback {
                measurement_ref(item, None, ctx)?.0
            } else {
                Value::Null
            };
            o.insert("readback".into(), rb);
            match value_type {
                ModbusValueType::Boolean => {
                    o.insert("valueType".into(), json!("boolean"));
                    o.insert(
                        "onLabel".into(),
                        json!(on_label.clone().unwrap_or_default()),
                    );
                    o.insert(
                        "offLabel".into(),
                        json!(off_label.clone().unwrap_or_default()),
                    );
                    o.insert("invert".into(), json!(invert));
                }
                ModbusValueType::Number => {
                    o.insert("valueType".into(), json!("number"));
                    o.insert("min".into(), json_number(min.unwrap_or(0.0)));
                    o.insert("max".into(), json_number(max.unwrap_or(65535.0)));
                    o.insert("step".into(), json_number(step.unwrap_or(1.0)));
                    if let Some(u) = unit.clone().or_else(|| item.unit().map(str::to_owned)) {
                        o.insert("unit".into(), json!(u));
                    }
                }
                ModbusValueType::Multi => {
                    o.insert("valueType".into(), json!("multi"));
                    let opts: Vec<Value> = options
                        .iter()
                        .map(|x| json!({ "value": x.value, "label": x.label, "icon": x.icon }))
                        .collect();
                    o.insert("options".into(), Value::Array(opts));
                }
            }
            Ok((Value::Object(o), item.title().de.clone()))
        }
    }
}

/// `POST /inventory/managedObjects` body of a floorplan: `{type: "lw_floorplan", name,
/// lw_floorplan: {description, icon, imageBinaryId, imageWidth, imageHeight, imageOpacity?,
/// widgets}}`. `image_binary_id` is the id returned by `POST /inventory/binaries`.
pub fn floorplan_body(
    layout: &FloorplanLayout,
    ctx: &FloorplanContext<'_>,
    image_binary_id: &str,
) -> AppResult<Value> {
    let widgets = layout
        .widgets
        .iter()
        .map(|w| widget_json(w, ctx))
        .collect::<AppResult<Vec<_>>>()?;
    let mut fp = Map::new();
    fp.insert("description".into(), json!(layout.description));
    fp.insert(
        "icon".into(),
        json!(layout.icon.clone().unwrap_or_else(|| "map".into())),
    );
    fp.insert("imageBinaryId".into(), json!(image_binary_id));
    fp.insert("imageWidth".into(), json!(layout.image_width));
    fp.insert("imageHeight".into(), json!(layout.image_height));
    if let Some(o) = layout.image_opacity {
        fp.insert("imageOpacity".into(), json_number(o));
    }
    fp.insert("widgets".into(), Value::Array(widgets));
    Ok(json!({ "type": FLOORPLAN_TYPE, "name": layout.name, "lw_floorplan": fp }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Library, loader};
    use crate::model::SlaveAssignment;
    use crate::payload::config::digital_input_entry;
    use crate::payload::naming::{assign_registers, assign_slaves};
    use std::path::Path;

    fn fixture() -> Library {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/library");
        loader::load(&root, "0.0.0").unwrap()
    }

    /// The alarms an item's effects raise belong to the item (a digital input has no native ones).
    #[test]
    fn effect_alarms_belong_to_the_item() {
        let item: LibraryItem = serde_json::from_value(serde_json::json!({
            "kind": "digitalInput", "id": "di-leak", "index": 4,
            "title": { "de": "Leck", "en": "Leak" }, "control": { "kind": "switch", "default": 0 },
            "effects": [
                { "kind": "alarm", "type": "lw_demo_leak", "severity": "MAJOR", "text": "Leck", "threshold": 1 },
                { "kind": "alarm", "type": "lw_demo_leak", "severity": "MAJOR", "text": "Leck", "threshold": 1 }
            ]
        }))
        .unwrap();
        let ctx = FloorplanContext {
            items: &[],
            slaves: &[],
            groups: &[],
            digital_inputs: &[],
        };
        assert_eq!(item_alarm_types(&item, &ctx).unwrap(), vec!["lw_demo_leak"]);
    }

    /// Slave + register assignment the way an apply would do it (fresh device).
    fn assignments(items: &[&LibraryItem]) -> Vec<SlaveAssignment> {
        let mut keys: Vec<&str> = Vec::new();
        for k in items.iter().filter_map(|i| i.slave_key()) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        let mut slaves = assign_slaves(&[], &keys).unwrap();
        for s in &mut slaves {
            let regs: Vec<(&str, _)> = items
                .iter()
                .filter(|i| i.slave_key() == Some(s.key.as_str()))
                .map(|i| (i.id(), i.register_type().unwrap()))
                .collect();
            s.registers = assign_registers(&s.registers, &regs).unwrap();
        }
        slaves
    }

    #[test]
    fn fixture_floorplan_resolves_to_plugin_widgets() {
        let lib = fixture();
        let demo = &lib.demos[0];
        let items = lib.demo_items(demo);
        let slaves = assignments(&items);
        let pairs: Vec<(&SlaveDefinition, &SlaveAssignment)> = slaves
            .iter()
            .map(|a| (lib.slave(&a.key).unwrap(), a))
            .collect();
        let inputs: Vec<DigitalInputEntry> = demo
            .digital_inputs
            .iter()
            .map(|c| digital_input_entry(c, &items).unwrap())
            .collect();
        let ctx = FloorplanContext {
            items: &items,
            slaves: &pairs,
            groups: &demo.dali_groups,
            digital_inputs: &inputs,
        };
        let layout = lib.floorplan(&demo.floorplans[0]).unwrap();
        let body = floorplan_body(layout, &ctx, "4711").unwrap();
        assert_eq!(body["type"], "lw_floorplan");
        assert_eq!(body["name"], "Erdgeschoss");
        let fp = &body["lw_floorplan"];
        assert_eq!(fp["imageBinaryId"], "4711");
        assert_eq!(
            (fp["imageWidth"].clone(), fp["imageHeight"].clone()),
            (json!(800), json!(400))
        );
        assert_eq!(fp["icon"], "map");
        let w = |id: &str| {
            fp["widgets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["id"] == id)
                .unwrap()
                .clone()
        };

        let freezer = w("w-freezer");
        assert_eq!(
            freezer["measurement"],
            json!({"fragment": "Tiefkühler Temperatur", "series": "freezer_temp"})
        );
        assert_eq!(freezer["unit"], "°C");
        assert_eq!(
            freezer["alarmTypes"],
            json!([
                "lw_modbus_20_timeoutError",
                "lw_demo_20_co2Leakage",
                "lw_demo_20_freezerDoor",
                "lw_demo_doorOpen"
            ])
        );
        assert_eq!(freezer["staleAfterMin"], 15);
        assert_eq!(
            freezer["ranges"][0],
            json!({"min": null, "max": -15, "level": "good"})
        );

        let lamp = w("w-lamp-1");
        assert_eq!(
            lamp["measurement"],
            json!({"fragment": "Licht 1", "series": "light-1"})
        );
        assert_eq!(lamp["title"], "Licht 1");
        assert_eq!(lamp["unit"], "%");
        assert_eq!(lamp["alarmTypes"].as_array().unwrap().len(), 14);
        assert_eq!(lamp["staleAfterMin"], Value::Null);

        let door = w("w-door");
        assert_eq!(
            door["measurement"],
            json!({"fragment": "Tür offen", "series": "di-door"})
        );
        assert_eq!(door["title"], "Tür offen");
        assert_eq!(door["alarmTypes"], json!([]));
        assert_eq!(door["display"]["kind"], "boolean");
        assert_eq!(door["display"]["onValue"], 1);

        assert_eq!(
            w("w-co2")["alarmTypes"],
            json!(["lw_demo_20_co2Leakage", "lw_powerFailure"])
        );

        let fan = w("w-hoodfan");
        assert_eq!(fan["title"], "Abluft");
        assert_eq!(
            fan["control"]["target"],
            json!({"source": "saved", "ref": "240:1", "slaveAddress": 240,
                   "registerAddress": 1, "type": "coil"})
        );
        assert_eq!(
            fan["control"]["readback"],
            json!({"fragment": "Abluft", "series": "hood_fan"})
        );
        assert_eq!(fan["control"]["valueType"], "boolean");
        assert_eq!(fan["control"]["invert"], false);

        let set = w("w-ac-set");
        assert_eq!(set["control"]["target"]["ref"], "239:40002");
        assert_eq!(set["control"]["target"]["type"], "register");
        assert_eq!(set["control"]["min"], 16);
        assert_eq!(set["confirm"], true);
        let mode = w("w-ac-mode");
        assert_eq!(mode["control"]["target"]["registerAddress"], 40001);
        assert_eq!(
            mode["control"]["options"][1],
            json!({"value": 1, "label": "Kühlen", "icon": "snowflake-o"})
        );
        assert_eq!(mode["title"], "Klima Modus");

        let dali = w("w-dali-shop");
        assert_eq!(dali["title"], "Shop");
        assert_eq!(
            dali["control"],
            json!({"type": "dali", "group": 1, "modes": ["off", "on", "auto"]})
        );
    }

    /// A lamp widget may show another of the lamp's series (library name), sent as
    /// `<id> <name>` (D10).
    #[test]
    fn dali_widget_series_maps_to_the_wire_name() {
        let lib = fixture();
        let demo = &lib.demos[0];
        let items = lib.demo_items(demo);
        let lamp = *items.iter().find(|i| i.id() == "light-1").unwrap();
        let ctx = FloorplanContext {
            items: &items,
            slaves: &[],
            groups: &[],
            digital_inputs: &[],
        };
        let (m, unit) = measurement_ref(lamp, Some("Lamp Status"), &ctx).unwrap();
        assert_eq!(
            m,
            json!({"fragment": "Licht 1", "series": "light-1 Lamp Status"})
        );
        assert_eq!(unit, None);
        let (m, unit) = measurement_ref(lamp, None, &ctx).unwrap();
        assert_eq!(m["series"], "light-1");
        assert_eq!(unit.as_deref(), Some("%"));
        assert!(measurement_ref(lamp, Some("Volt"), &ctx).is_err());
    }

    #[test]
    fn unresolvable_references_fail() {
        let lib = fixture();
        let demo = &lib.demos[0];
        let items = lib.demo_items(demo);
        let ctx = FloorplanContext {
            items: &items,
            slaves: &[],
            groups: &[],
            digital_inputs: &[],
        };
        // Without slave assignments a Modbus widget cannot be resolved.
        let layout = lib.floorplan(&demo.floorplans[0]).unwrap();
        assert!(floorplan_body(layout, &ctx, "1").is_err());
    }

    /// A DALI control whose group is missing on the device fails the apply (D6: groups are
    /// edited on the device), a DI widget without a device entry takes the item title.
    #[test]
    fn missing_group_fails_and_di_label_falls_back() {
        let lib = fixture();
        let demo = &lib.demos[0];
        let items = lib.demo_items(demo);
        let slaves = assignments(&items);
        let pairs: Vec<(&SlaveDefinition, &SlaveAssignment)> = slaves
            .iter()
            .map(|a| (lib.slave(&a.key).unwrap(), a))
            .collect();
        let ctx = FloorplanContext {
            items: &items,
            slaves: &pairs,
            groups: &[],
            digital_inputs: &[],
        };
        let layout = lib.floorplan(&demo.floorplans[0]).unwrap();
        let err = floorplan_body(layout, &ctx, "1").unwrap_err().to_string();
        assert!(err.contains("group 1"), "{err}");

        let ctx = FloorplanContext {
            groups: &demo.dali_groups,
            ..ctx
        };
        let body = floorplan_body(layout, &ctx, "1").unwrap();
        let door = body["lw_floorplan"]["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == "w-door")
            .unwrap()
            .clone();
        assert_eq!(door["title"], "Tür offen");
    }

    /// The DI type, title (widget title and fragment) and counter unit follow the device
    /// entry, not the library item.
    #[test]
    fn di_widget_follows_the_device_entry() {
        use crate::model::DigitalInputType;
        let lib = fixture();
        let demo = &lib.demos[0];
        let items = lib.demo_items(demo);
        let gas = *items.iter().find(|i| i.id() == "gas-meter").unwrap();
        let mut inputs: Vec<DigitalInputEntry> = demo
            .digital_inputs
            .iter()
            .map(|c| digital_input_entry(c, &items).unwrap())
            .collect();
        inputs[2].settings.unit = Some("kWh".into());
        inputs[2].title = "Gas Küche".into();
        let ctx = FloorplanContext {
            items: &items,
            slaves: &[],
            groups: &[],
            digital_inputs: &inputs,
        };
        let (m, unit) = measurement_ref(gas, None, &ctx).unwrap();
        assert_eq!(m, json!({"fragment": "Gas Küche", "series": "gas-meter"}));
        assert_eq!(unit.as_deref(), Some("kWh"));
        assert_eq!(default_title(gas, &ctx), "Gas Küche");
        let mut as_value = inputs.clone();
        as_value[2].settings.input_type = DigitalInputType::Measurement;
        let ctx = FloorplanContext {
            digital_inputs: &as_value,
            ..ctx
        };
        let (m, unit) = measurement_ref(gas, None, &ctx).unwrap();
        assert_eq!(m["series"], "gas-meter");
        assert_eq!(unit, None);
    }
}
