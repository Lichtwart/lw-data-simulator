//! Export a demo device as a demo pack (PRD §13, AGENTS.md "Exporting a device by hand").
//!
//! Writes a `library/`-shaped tree into the target folder:
//! - `demos/<demoId>.json`: `lw_demo` tags / includes / excludes / lamp count / interval,
//!   `lw_demo.daliGroups` → `daliGroups`, `lw_demo.digitalInputs` → `digitalInputs`, the library
//!   demo's scenarios plus a `start` scenario carrying generator overrides of library items;
//! - `items/<demoId>.json` (only if needed): custom slaves / items (generator overrides applied,
//!   DI titles from the device config) and DI items created for inputs without one;
//! - `floorplans/<demoId>/<name>.json` + image: widgets mapped back to item references
//!   (`measurement {fragment, series}` → `itemId` by the series, which is the item id (D10);
//!   Modbus targets → `itemId`, alarm types → alarm refs minus the ones `autoAlarms` adds),
//!   `deviceId` dropped.
//!
//! The result applies back to an equal device configuration (tested in `device::tests`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::setup::{self, ResolvedSetup};
use super::{crud, demo_state, floorplan};
use crate::c8y::C8yClient;
use crate::error::{AppError, AppResult};
use crate::library::Library;
use crate::model::{
    AlarmRef, DemoPack, DeviceConfig, DigitalInputConfig, DigitalInputType, ExportRequest,
    ExportResult, FloorplanLayout, FloorplanWidget, ItemLibraryFile, LibraryItem, LocalizedText,
    LwDemoState, ModbusControlOption, ModbusValueType, Scenario, ScenarioStep, SensorDisplay,
    WidgetControl, WidgetMeasurement, WidgetRange,
};
use crate::payload::alarm::device_custom_alarm;
use crate::payload::floorplan::{FloorplanContext, item_alarm_types};
use crate::payload::naming::{
    DALI_VALUE_SERIES, MAX_NAME_LEN, dali_library_series, item_naming_with_di, slave_alarm_type,
    slave_timeout_alarm_type,
};

/// `[a-z0-9-]+`.
pub fn is_valid_demo_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// File-name slug of a floorplan name.
fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'ä' => "ae".to_owned(),
            'ö' => "oe".to_owned(),
            'ü' => "ue".to_owned(),
            'ß' => "ss".to_owned(),
            c if c.is_ascii_alphanumeric() => c.to_string(),
            _ => "-".to_owned(),
        };
        s.push_str(&c);
    }
    let s: String = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "floorplan".into() } else { s }
}

/// Image extension from its bytes.
fn sniff_extension(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else {
        "svg"
    }
}

/// One floorplan read from the device: managed object + image bytes.
pub struct DeviceFloorplan {
    pub mo: Value,
    pub image: Vec<u8>,
}

/// Everything export needs from the device.
pub struct ExportInput<'a> {
    pub library: &'a Library,
    pub demo_id: &'a str,
    pub device_name: &'a str,
    pub device_description: Option<&'a str>,
    pub state: &'a LwDemoState,
    pub config: &'a DeviceConfig,
    pub floorplans: &'a [DeviceFloorplan],
}

/// A file to write: path relative to the target folder + content.
pub struct ExportFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

fn json_file<T: serde::Serialize>(path: String, v: &T) -> AppResult<ExportFile> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    Ok(ExportFile {
        path,
        bytes: s.into_bytes(),
    })
}

/// A fresh item id `<base>` (shortened to 19 chars) not used by the library or `taken`.
fn unique_id(library: &Library, taken: &[String], base: &str) -> String {
    let short: String = base.chars().take(MAX_NAME_LEN).collect();
    let free = |id: &str| library.item(id).is_none() && !taken.iter().any(|t| t == id);
    if free(&short) {
        return short;
    }
    for n in 2.. {
        let suffix = format!("-{n}");
        let id: String = base
            .chars()
            .take(MAX_NAME_LEN - suffix.len())
            .collect::<String>()
            + &suffix;
        if free(&id) {
            return id;
        }
    }
    unreachable!()
}

fn new_di_item(
    id: String,
    index: u32,
    title: &str,
    counter: Option<Option<String>>,
) -> LibraryItem {
    let title = if title.is_empty() {
        format!("DI {index:02}")
    } else {
        title.to_owned()
    };
    let title = LocalizedText {
        de: title.clone(),
        en: title,
    };
    match counter {
        Some(unit) => LibraryItem::ImpulseCounter {
            id,
            index,
            title,
            tags: Vec::new(),
            unit,
            control: Some(crate::model::Control::Counter { default: 0.0 }),
            generator: None,
            effects: Vec::new(),
        },
        None => LibraryItem::DigitalInput {
            id,
            index,
            title,
            tags: Vec::new(),
            unit: None,
            control: Some(crate::model::Control::Switch { default: 0.0 }),
            generator: None,
            effects: Vec::new(),
        },
    }
}

fn set_title_de(item: &mut LibraryItem, de: &str) {
    match item {
        LibraryItem::DigitalInput { title, .. } | LibraryItem::ImpulseCounter { title, .. } => {
            title.de = de.to_owned();
        }
        _ => {}
    }
}

fn set_generator(item: &mut LibraryItem, g: crate::model::GeneratorConfig) {
    match item {
        LibraryItem::Dali { generator, .. }
        | LibraryItem::DaliBrightness { generator, .. }
        | LibraryItem::DaliMotion { generator, .. }
        | LibraryItem::DigitalInput { generator, .. }
        | LibraryItem::ImpulseCounter { generator, .. }
        | LibraryItem::System { generator, .. }
        | LibraryItem::Modbus { generator, .. } => *generator = Some(g),
        LibraryItem::CustomAlarm { .. } => {}
    }
}

/// Reverse lookup of the resolved setup (Cumulocity names → item references).
struct Reverse<'a> {
    resolved: &'a ResolvedSetup,
    inputs: &'a [crate::model::DigitalInputEntry],
}

impl Reverse<'_> {
    /// `(item id, library series name if not the default)` for a measurement `{fragment,
    /// series}`. The series names the item (its id, D10), so a title edited after the floorplan
    /// was made (a different fragment) still maps back.
    fn measurement(&self, _fragment: &str, series: &str) -> Option<(String, Option<String>)> {
        for item in &self.resolved.items {
            let slave = item
                .slave_key()
                .and_then(|k| self.resolved.slaves.iter().find(|(d, _)| d.key == k))
                .map(|(_, a)| a);
            let di = match item {
                LibraryItem::DigitalInput { index, .. }
                | LibraryItem::ImpulseCounter { index, .. } => {
                    self.inputs.iter().find(|e| e.index == *index)
                }
                _ => None,
            };
            let Some(n) = item_naming_with_di(item, slave, di) else {
                continue;
            };
            if n.series.iter().any(|s| s.name == series) {
                let library_series = if n.series.len() == 1 {
                    None
                } else {
                    Some(dali_library_series(series)).filter(|s| *s != DALI_VALUE_SERIES)
                };
                return Some((item.id().to_owned(), library_series.map(str::to_owned)));
            }
        }
        None
    }

    /// Writable Modbus item behind a control target.
    fn target(&self, slave_address: u32, register_address: u32, typ: &str) -> Option<String> {
        for (_, a) in &self.resolved.slaves {
            if a.address != slave_address {
                continue;
            }
            for r in &a.registers {
                if r.address == register_address
                    && r.register_type.write_operation_type() == Some(typ)
                {
                    return Some(r.item_id.clone());
                }
            }
        }
        None
    }

    /// Alarm ref for a literal alarm type.
    fn alarm_ref(&self, alarm_type: &str) -> AlarmRef {
        for (def, a) in &self.resolved.slaves {
            if alarm_type == slave_timeout_alarm_type(a.id) {
                return AlarmRef::SlaveTimeout {
                    slave_key: def.key.clone(),
                };
            }
            for al in &def.alarms {
                if alarm_type == slave_alarm_type(a.id, &al.name) {
                    return AlarmRef::SlaveAlarm {
                        slave_key: def.key.clone(),
                        alarm_name: al.name.clone(),
                    };
                }
            }
        }
        for item in &self.resolved.items {
            if device_custom_alarm(item).is_some_and(|s| s.alarm_type == alarm_type) {
                return AlarmRef::Item {
                    item_id: item.id().to_owned(),
                };
            }
        }
        AlarmRef::Type {
            alarm_type: alarm_type.to_owned(),
        }
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

/// Map one plugin widget back to a layout widget; `Err(reason)` drops it with a warning.
fn widget_back(
    w: &Value,
    rev: &Reverse<'_>,
    ctx: &FloorplanContext<'_>,
    device_id: Option<&str>,
) -> Result<FloorplanWidget, String> {
    let id = s(w, "id").unwrap_or_default();
    if let (Some(dev), Some(own)) = (s(w, "deviceId"), device_id)
        && dev != own
    {
        return Err(format!("widget {id} shows another device ({dev})"));
    }
    let x = num(w, "x").unwrap_or(0.0);
    let y = num(w, "y").unwrap_or(0.0);
    let locked = w.get("locked").and_then(Value::as_bool).unwrap_or(false);
    let icon = s(w, "icon").unwrap_or_default();
    match w.get("kind").and_then(Value::as_str) {
        Some("sensor") => {
            let fragment = w.pointer("/measurement/fragment").and_then(Value::as_str);
            let series = w.pointer("/measurement/series").and_then(Value::as_str);
            let (Some(fragment), Some(series)) = (fragment, series) else {
                return Err(format!("widget {id} has no measurement"));
            };
            let (item_id, series) = rev
                .measurement(fragment, series)
                .ok_or_else(|| format!("widget {id}: no item for {fragment}.{series}"))?;
            let item = rev.resolved.item(&item_id).expect("resolved item");
            let auto = item_alarm_types(item, ctx).unwrap_or_default();
            let types = strings(w.get("alarmTypes"));
            let auto_alarms = auto.iter().all(|t| types.contains(t));
            let alarms = types
                .iter()
                .filter(|t| !(auto_alarms && auto.contains(t)))
                .map(|t| rev.alarm_ref(t))
                .collect();
            let display = match w.get("display") {
                Some(d) if !d.is_null() => serde_json::from_value::<SensorDisplay>(d.clone())
                    .map(Some)
                    .map_err(|e| format!("widget {id}: display {e}"))?,
                _ => None,
            };
            let ranges = w
                .get("ranges")
                .cloned()
                .map(serde_json::from_value::<Vec<WidgetRange>>)
                .transpose()
                .map_err(|e| format!("widget {id}: ranges {e}"))?
                .unwrap_or_default();
            // A DI widget whose title equals the device's DI title gets it back on apply.
            let di_title = match item {
                LibraryItem::DigitalInput { index, .. }
                | LibraryItem::ImpulseCounter { index, .. } => ctx
                    .digital_inputs
                    .iter()
                    .find(|e| e.index == *index && !e.title.is_empty())
                    .map(|e| e.title.clone()),
                _ => None,
            };
            let title = s(w, "title").filter(|t| di_title.as_deref() != Some(t.as_str()));
            Ok(FloorplanWidget::Sensor {
                id,
                x,
                y,
                locked,
                icon,
                title,
                measurement: WidgetMeasurement { item_id, series },
                unit: s(w, "unit"),
                decimals: w
                    .get("decimals")
                    .and_then(Value::as_u64)
                    .and_then(|d| u32::try_from(d).ok()),
                display,
                ranges,
                auto_alarms,
                alarms,
                stale_after_min: w
                    .get("staleAfterMin")
                    .and_then(Value::as_f64)
                    .map(|v| v.max(0.0) as u32),
            })
        }
        Some("alarm") => Ok(FloorplanWidget::Alarm {
            id,
            x,
            y,
            locked,
            icon,
            text: s(w, "text").unwrap_or_default(),
            note: s(w, "note"),
            alarms: strings(w.get("alarmTypes"))
                .iter()
                .map(|t| rev.alarm_ref(t))
                .collect(),
        }),
        Some("control") => {
            let c = w.get("control").cloned().unwrap_or(Value::Null);
            let control = match c.get("type").and_then(Value::as_str) {
                Some("dali") => WidgetControl::Dali {
                    group: c
                        .get("group")
                        .and_then(Value::as_u64)
                        .and_then(|g| u32::try_from(g).ok())
                        .ok_or_else(|| format!("widget {id}: no group"))?,
                    modes: serde_json::from_value(c.get("modes").cloned().unwrap_or_default())
                        .map_err(|e| format!("widget {id}: modes {e}"))?,
                },
                Some("modbus") => {
                    let t = c.get("target").cloned().unwrap_or(Value::Null);
                    let sa = t.get("slaveAddress").and_then(Value::as_u64).unwrap_or(0) as u32;
                    let ra = t
                        .get("registerAddress")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32;
                    let typ = t.get("type").and_then(Value::as_str).unwrap_or("register");
                    let item_id = rev
                        .target(sa, ra, typ)
                        .ok_or_else(|| format!("widget {id}: no item for {typ} {sa}:{ra}"))?;
                    let value_type = match c.get("valueType").and_then(Value::as_str) {
                        Some("boolean") => ModbusValueType::Boolean,
                        Some("multi") => ModbusValueType::Multi,
                        _ => ModbusValueType::Number,
                    };
                    let options: Vec<ModbusControlOption> = c
                        .get("options")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|e| format!("widget {id}: options {e}"))?
                        .unwrap_or_default();
                    let is = |vt| value_type == vt;
                    WidgetControl::Modbus {
                        item_id,
                        readback: c.get("readback").is_some_and(|r| !r.is_null()),
                        value_type,
                        on_label: s(&c, "onLabel").filter(|_| is(ModbusValueType::Boolean)),
                        off_label: s(&c, "offLabel").filter(|_| is(ModbusValueType::Boolean)),
                        invert: is(ModbusValueType::Boolean)
                            && c.get("invert").and_then(Value::as_bool).unwrap_or(false),
                        min: num(&c, "min").filter(|_| is(ModbusValueType::Number)),
                        max: num(&c, "max").filter(|_| is(ModbusValueType::Number)),
                        step: num(&c, "step").filter(|_| is(ModbusValueType::Number)),
                        unit: s(&c, "unit").filter(|_| is(ModbusValueType::Number)),
                        options,
                    }
                }
                other => return Err(format!("widget {id}: control type {other:?}")),
            };
            Ok(FloorplanWidget::Control {
                id,
                x,
                y,
                locked,
                icon,
                title: s(w, "title"),
                confirm: w.get("confirm").and_then(Value::as_bool).unwrap_or(false),
                control,
            })
        }
        other => Err(format!("widget {id}: kind {other:?}")),
    }
}

/// Build all export files (pure). Returns files + warnings.
pub fn build(
    input: &ExportInput<'_>,
    device_id: &str,
) -> AppResult<(Vec<ExportFile>, Vec<String>)> {
    let lib = input.library;
    let id = input.demo_id;
    if !is_valid_demo_id(id) {
        return Err(AppError::InvalidInput(format!("demo id {id} ([a-z0-9-])")));
    }
    let st = input.state;
    let mut warnings = Vec::new();
    let resolved = setup::resolve(lib, st)?;

    // ── items: custom ones (generators applied) + DI items created for inputs ──
    let mut custom: Vec<LibraryItem> = st.custom_items.clone();
    let mut start_steps: Vec<ScenarioStep> = Vec::new();
    for (item_id, g) in &st.generators {
        if let Some(c) = custom.iter_mut().find(|c| c.id() == item_id) {
            set_generator(c, g.clone());
        } else if resolved.item(item_id).is_some() {
            start_steps.push(ScenarioStep::Generator {
                item_id: item_id.clone(),
                generator: g.clone(),
            });
        }
    }
    let mut includes = st.includes.clone();
    let mut digital_inputs = Vec::new();
    for e in &input.config.digital_inputs {
        let counter = e.settings.input_type == DigitalInputType::ImpulseCounter;
        let found = resolved.items.iter().find(|i| {
            i.index() == Some(e.index)
                && matches!(
                    (i, counter),
                    (LibraryItem::ImpulseCounter { .. }, true)
                        | (LibraryItem::DigitalInput { .. }, false)
                )
        });
        let item_id = match found {
            Some(item) => {
                if let Some(c) = custom.iter_mut().find(|c| c.id() == item.id()) {
                    if !e.title.is_empty() {
                        set_title_de(c, &e.title);
                    }
                } else if !e.title.is_empty() && item.title().de != e.title {
                    warnings.push(format!(
                        "digital input {}: title \"{}\" differs from library item {} (\"{}\"); \
                         edit the item to keep it",
                        e.index,
                        e.title,
                        item.id(),
                        item.title().de
                    ));
                }
                item.id().to_owned()
            }
            None => {
                let taken: Vec<String> = custom.iter().map(|c| c.id().to_owned()).collect();
                let new_id = unique_id(lib, &taken, &format!("{id}-di{:02}", e.index));
                custom.push(new_di_item(
                    new_id.clone(),
                    e.index,
                    &e.title,
                    counter.then(|| e.settings.unit.clone()),
                ));
                new_id
            }
        };
        digital_inputs.push(DigitalInputConfig {
            index: e.index,
            item_id,
            settings: e.settings.clone(),
        });
    }
    for c in &custom {
        if !includes.iter().any(|i| i == c.id()) {
            includes.push(c.id().to_owned());
        }
    }

    // ── scenarios ──
    let library_demo = st.demo_id.as_deref().and_then(|d| lib.demo(d));
    let mut scenarios: Vec<Scenario> = library_demo
        .map(|d| d.scenarios.clone())
        .unwrap_or_default();
    if !start_steps.is_empty() {
        match scenarios.iter_mut().find(|s| s.start) {
            Some(s) => s.set.extend(start_steps),
            None => scenarios.push(Scenario {
                id: "start".into(),
                name: LocalizedText {
                    de: "Start".into(),
                    en: "Start".into(),
                },
                start: true,
                set: start_steps,
            }),
        }
    }

    // ── floorplans ──
    let mut files = Vec::new();
    let mut fp_paths = Vec::new();
    let export_setup = {
        // Floorplan context of the device as it is now (custom DI titles etc.).
        let mut r = resolved.clone();
        for c in &custom {
            if let Some(i) = r.items.iter_mut().find(|i| i.id() == c.id()) {
                *i = c.clone();
            } else {
                r.items.push(c.clone());
            }
        }
        r
    };
    let items = export_setup.item_refs();
    let slaves = export_setup.slave_refs();
    let ctx = FloorplanContext {
        items: &items,
        slaves: &slaves,
        groups: &input.config.groups,
        digital_inputs: &input.config.digital_inputs,
    };
    let rev = Reverse {
        resolved: &export_setup,
        inputs: &input.config.digital_inputs,
    };
    let mut used_slugs: Vec<String> = Vec::new();
    for fp in input.floorplans {
        let name = s(&fp.mo, "name").unwrap_or_else(|| "Floorplan".into());
        let mut base = slug(&name);
        if used_slugs.contains(&base) {
            base = format!("{base}-{}", used_slugs.len() + 1);
        }
        used_slugs.push(base.clone());
        let data = fp.mo.get("lw_floorplan").cloned().unwrap_or(Value::Null);
        let image = format!("{base}.{}", sniff_extension(&fp.image));
        let mut widgets = Vec::new();
        for w in data
            .get("widgets")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            match widget_back(&w, &rev, &ctx, Some(device_id)) {
                Ok(w) => widgets.push(w),
                Err(e) => warnings.push(format!("floorplan \"{name}\": {e}, widget dropped")),
            }
        }
        let layout = FloorplanLayout {
            file_type: "lw-demo-floorplan".into(),
            schema_version: 1,
            name: name.clone(),
            description: s(&data, "description").unwrap_or_default(),
            icon: s(&data, "icon"),
            image: image.clone(),
            image_width: data.get("imageWidth").and_then(Value::as_u64).unwrap_or(0) as u32,
            image_height: data.get("imageHeight").and_then(Value::as_u64).unwrap_or(0) as u32,
            image_opacity: num(&data, "imageOpacity"),
            widgets,
        };
        let rel = format!("floorplans/{id}/{base}.json");
        files.push(json_file(rel.clone(), &layout)?);
        files.push(ExportFile {
            path: format!("floorplans/{id}/{image}"),
            bytes: fp.image.clone(),
        });
        fp_paths.push(rel);
    }

    // ── demo ──
    let lamp_count = st.dali_lamp_count.clamp(1, 64);
    if st.dali_lamp_count == 0 {
        warnings.push("daliLampCount 0 exported as 1".into());
    }
    let demo = DemoPack {
        file_type: "lw-demo".into(),
        schema_version: 1,
        id: id.to_owned(),
        name: LocalizedText {
            de: input.device_name.to_owned(),
            en: input.device_name.to_owned(),
        },
        description: library_demo
            .map(|d| d.description.clone())
            .unwrap_or_else(|| {
                let d = input.device_description.unwrap_or_default().to_owned();
                LocalizedText {
                    de: d.clone(),
                    en: d,
                }
            }),
        tags: st.tags.clone(),
        includes,
        excludes: st.excludes.clone(),
        dali_lamp_count: lamp_count,
        dali_groups: input.config.groups.clone(),
        digital_inputs,
        send_interval_sec: st.send_interval_sec,
        scenarios,
        floorplans: fp_paths,
    };
    files.insert(0, json_file(format!("demos/{id}.json"), &demo)?);
    if !custom.is_empty() || !st.custom_slaves.is_empty() {
        let items_file = ItemLibraryFile {
            file_type: "lw-demo-items".into(),
            schema_version: 1,
            slaves: st.custom_slaves.clone(),
            items: custom,
        };
        files.insert(1, json_file(format!("items/{id}.json"), &items_file)?);
    }
    Ok((files, warnings))
}

/// Write `files` below `root`.
pub fn write_files(root: &Path, files: &[ExportFile]) -> AppResult<Vec<String>> {
    let mut written = Vec::new();
    for f in files {
        let path: PathBuf = root.join(&f.path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| AppError::Io(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&path, &f.bytes)
            .map_err(|e| AppError::Io(format!("{}: {e}", path.display())))?;
        written.push(f.path.clone());
    }
    Ok(written)
}

pub async fn export(
    client: &C8yClient,
    library: &Library,
    req: &ExportRequest,
) -> AppResult<ExportResult> {
    if !is_valid_demo_id(&req.demo_id) {
        return Err(AppError::InvalidInput(format!(
            "demo id {} ([a-z0-9-])",
            req.demo_id
        )));
    }
    let root = PathBuf::from(&req.target_dir);
    if !root.is_dir() {
        return Err(AppError::Io(format!("{}: not a folder", root.display())));
    }
    let mo = crud::load_demo_mo(client, &req.device_id).await?;
    let state = demo_state::read(&mo)?
        .ok_or_else(|| AppError::NotFound(format!("lw_demo on {}", req.device_id)))?;
    let config = setup::device_config(library, &state);
    let mut floorplans = Vec::new();
    for fp in floorplan::list(client, &req.device_id).await? {
        let binary = fp.pointer("/lw_floorplan/imageBinaryId").and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        });
        let image = match binary {
            Some(b) => client.download_binary(&b).await?,
            None => Vec::new(),
        };
        floorplans.push(DeviceFloorplan { mo: fp, image });
    }
    let input = ExportInput {
        library,
        demo_id: &req.demo_id,
        device_name: mo
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&req.demo_id),
        device_description: mo.get("description").and_then(Value::as_str),
        state: &state,
        config: &config,
        floorplans: &floorplans,
    };
    let (files, warnings) = build(&input, &req.device_id)?;
    let files_written = write_files(&root, &files)?;
    Ok(ExportResult {
        path: root.display().to_string(),
        files_written,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_ids() {
        assert_eq!(slug("Erdgeschoss Süd"), "erdgeschoss-sued");
        assert_eq!(slug("  "), "floorplan");
        assert!(is_valid_demo_id("gas-station-2"));
        assert!(!is_valid_demo_id("Gas"));
        assert_eq!(sniff_extension(b"\x89PNG...."), "png");
        assert_eq!(sniff_extension(b"<svg"), "svg");
    }
}
