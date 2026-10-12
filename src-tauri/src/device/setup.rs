//! Resolving a device setup (`lw_demo` + library) into items and slaves, slave / register
//! assignment, lenient parsing of the device's `lw_config*` fragments, digests and versions.
//!
//! Shared by apply, upgrade, export, `update_demo_setup`, the fault catalog and the session, so
//! every place resolves a device exactly the same way (DECISIONS D6).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::library::{Library, select};
use crate::model::{
    DaliGroup, DemoPack, DeviceConfig, DigitalInputEntry, DigitalInputSettings, DigitalInputType,
    LibraryItem, LwDemoState, ModbusRegisterType, SlaveAssignment, SlaveDefinition,
};
use crate::payload::config::{digital_input_entry, digital_inputs_fragment, groups_fragment};
use crate::payload::naming::{assign_registers, assign_slaves};

/// Items and slaves of a device setup (owned, so a session can keep them).
#[derive(Debug, Clone, Default)]
pub struct ResolvedSetup {
    /// Library selection (`tags ∪ includes − excludes`, library order) followed by the device's
    /// custom items (minus `excludes`).
    pub items: Vec<LibraryItem>,
    /// Slaves referenced by the selected Modbus items (order of first use) with their assignment.
    pub slaves: Vec<(SlaveDefinition, SlaveAssignment)>,
    /// Assignments that are not selected right now (see `LwDemoState::retired_slaves`): store
    /// them back with `slaves` so numbers are never reused.
    pub retired: Vec<SlaveAssignment>,
}

impl ResolvedSetup {
    pub fn item_refs(&self) -> Vec<&LibraryItem> {
        self.items.iter().collect()
    }

    pub fn slave_refs(&self) -> Vec<(&SlaveDefinition, &SlaveAssignment)> {
        self.slaves.iter().map(|(d, a)| (d, a)).collect()
    }

    pub fn item(&self, id: &str) -> Option<&LibraryItem> {
        self.items.iter().find(|i| i.id() == id)
    }
}

/// Selected items of a setup: library selection + custom items (not excluded).
pub fn select_items(library: &Library, demo: &LwDemoState) -> Vec<LibraryItem> {
    let mut items: Vec<LibraryItem> =
        select::select_by(&library.items, &demo.tags, &demo.includes, &demo.excludes)
            .into_iter()
            .cloned()
            .collect();
    for c in &demo.custom_items {
        if !demo.excludes.iter().any(|e| e == c.id()) && !items.iter().any(|i| i.id() == c.id()) {
            items.push(c.clone());
        }
    }
    items
}

/// Slave definition by key: library first, then the device's custom slaves.
pub fn slave_definition<'a>(
    library: &'a Library,
    demo: &'a LwDemoState,
    key: &str,
) -> Option<&'a SlaveDefinition> {
    library
        .slave(key)
        .or_else(|| demo.custom_slaves.iter().find(|s| s.key == key))
}

/// Slave keys used by `items`, in order of first use.
pub fn slave_keys(items: &[LibraryItem]) -> Vec<&str> {
    let mut keys: Vec<&str> = Vec::new();
    for k in items.iter().filter_map(LibraryItem::slave_key) {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys
}

/// Slave ids / addresses and register addresses for `items`, keeping every existing assignment
/// (D6: never renumbered; new slaves count down from 20 / 240, new registers take the lowest
/// free address of their type). `retired` are assignments of slaves / registers that are not
/// selected (any more): their numbers stay taken and come back when the item is selected again.
///
/// Returns `(active, retired)`: the assignments for `items`, and everything of `existing` /
/// `retired` that is not selected, to be stored in `LwDemoState::retired_slaves`.
pub fn assign_all(
    existing: &[SlaveAssignment],
    retired: &[SlaveAssignment],
    items: &[LibraryItem],
) -> AppResult<(Vec<SlaveAssignment>, Vec<SlaveAssignment>)> {
    // Everything ever assigned, per slave key.
    let mut pool: Vec<SlaveAssignment> = existing.to_vec();
    for r in retired {
        match pool.iter_mut().find(|p| p.key == r.key) {
            Some(p) => {
                for reg in &r.registers {
                    if !p
                        .registers
                        .iter()
                        .any(|x| x.item_id == reg.item_id && x.register_type == reg.register_type)
                    {
                        p.registers.push(reg.clone());
                    }
                }
            }
            None => pool.push(r.clone()),
        }
    }
    let keys = slave_keys(items);
    let mut slaves = assign_slaves(&pool, &keys)?;
    for s in &mut slaves {
        let regs: Vec<(&str, ModbusRegisterType)> = items
            .iter()
            .filter(|i| i.slave_key() == Some(s.key.as_str()))
            .filter_map(|i| i.register_type().map(|t| (i.id(), t)))
            .collect();
        s.registers = assign_registers(&s.registers, &regs)?;
    }
    let mut out_retired = Vec::new();
    for p in &pool {
        match slaves.iter().find(|s| s.key == p.key) {
            None => out_retired.push(p.clone()),
            Some(active) => {
                let registers: Vec<_> =
                    p.registers
                        .iter()
                        .filter(|r| {
                            !active.registers.iter().any(|a| {
                                a.item_id == r.item_id && a.register_type == r.register_type
                            })
                        })
                        .cloned()
                        .collect();
                if !registers.is_empty() {
                    out_retired.push(SlaveAssignment {
                        registers,
                        ..p.clone()
                    });
                }
            }
        }
    }
    Ok((slaves, out_retired))
}

/// Resolve a setup: select items, look up slave definitions, assign (keeping `demo.slaves` and
/// `demo.retired_slaves`).
pub fn resolve(library: &Library, demo: &LwDemoState) -> AppResult<ResolvedSetup> {
    let items = select_items(library, demo);
    let (assignments, retired) = assign_all(&demo.slaves, &demo.retired_slaves, &items)?;
    let mut slaves = Vec::with_capacity(assignments.len());
    for a in assignments {
        let def = slave_definition(library, demo, &a.key)
            .ok_or_else(|| AppError::NotFound(format!("slave {}", a.key)))?
            .clone();
        slaves.push((def, a));
    }
    Ok(ResolvedSetup {
        items,
        slaves,
        retired,
    })
}

impl ResolvedSetup {
    /// Store the assignments (active and retired) in `state`.
    pub fn store_assignments(&self, state: &mut LwDemoState) {
        state.slaves = self.slaves.iter().map(|(_, a)| a.clone()).collect();
        state.retired_slaves = self.retired.clone();
    }
}

// ───────────────────────────── export round trip (finding 14) ─────────────────────────────

/// What [`drop_library_duplicates`] did.
#[derive(Debug, Clone, Default)]
pub struct CustomDedupe {
    /// Ids of custom slaves / items that the library has now.
    pub now_in_library: Vec<String>,
    /// Entries whose library version differs from the custom one (the library one is used).
    pub warnings: Vec<String>,
}

/// Custom slaves / items whose id now exists in the library (an exported pack was released) are
/// dropped from the device: the library version is used. Equal ones silently, different ones
/// with a warning. An item that was selected only as a custom item is added to `includes`, so
/// the device does not lose it. `known`: only entries that were custom on the device before
/// (a patch that newly introduces a colliding id is an error, not a release).
pub fn drop_library_duplicates(
    library: &Library,
    state: &mut LwDemoState,
    known: Option<&std::collections::BTreeSet<String>>,
) -> CustomDedupe {
    let is_known = |id: &str| known.is_none_or(|k| k.contains(id));
    let mut out = CustomDedupe::default();
    let mut keep_items = Vec::new();
    for c in std::mem::take(&mut state.custom_items) {
        match library.item(c.id()).filter(|_| is_known(c.id())) {
            None => keep_items.push(c),
            Some(l) => {
                if digest(l) != digest(&c) {
                    out.warnings.push(format!(
                        "custom item {} is in the library now; the library version is used",
                        c.id()
                    ));
                }
                out.now_in_library.push(c.id().to_owned());
                let id = c.id().to_owned();
                let selected = select::select_by(
                    &library.items,
                    &state.tags,
                    &state.includes,
                    &state.excludes,
                )
                .iter()
                .any(|i| i.id() == id);
                if !selected && !state.excludes.contains(&id) {
                    state.includes.push(id);
                }
            }
        }
    }
    state.custom_items = keep_items;
    let mut keep_slaves = Vec::new();
    for c in std::mem::take(&mut state.custom_slaves) {
        match library.slave(&c.key).filter(|_| is_known(&c.key)) {
            None => keep_slaves.push(c),
            Some(l) => {
                if digest(l) != digest(&c) {
                    out.warnings.push(format!(
                        "custom slave {} is in the library now; the library version is used",
                        c.key
                    ));
                }
                out.now_in_library.push(c.key.clone());
            }
        }
    }
    state.custom_slaves = keep_slaves;
    out
}

// ───────────────────────────── demo-pack level digests (finding 12) ─────────────────────────────

/// Fields of a demo pack that are copied to the device at apply time.
pub const DEMO_FIELDS: &[&str] = &[
    "tags",
    "includes",
    "excludes",
    "daliLampCount",
    "daliGroups",
    "digitalInputs",
    "sendIntervalSec",
];

/// Digests of the demo pack's values as they are written to the device (groups and inputs go
/// through the `lw_config*` fragment and back, so they compare with what the device returns).
pub fn demo_digests(
    demo: &DemoPack,
    items: &[&LibraryItem],
) -> AppResult<BTreeMap<String, String>> {
    let inputs = demo
        .digital_inputs
        .iter()
        .map(|c| digital_input_entry(c, items))
        .collect::<AppResult<Vec<_>>>()?;
    let groups = parse_groups(Some(&groups_fragment(&demo.dali_groups)));
    let inputs = parse_digital_inputs(Some(&digital_inputs_fragment(&inputs)));
    Ok(device_digests(
        &demo.tags,
        &demo.includes,
        &demo.excludes,
        demo.dali_lamp_count,
        demo.send_interval_sec,
        &groups,
        &inputs,
    ))
}

/// Digests of the same fields as the device has them now.
pub fn current_digests(state: &LwDemoState, config: &DeviceConfig) -> BTreeMap<String, String> {
    device_digests(
        &state.tags,
        &state.includes,
        &state.excludes,
        state.dali_lamp_count,
        state.send_interval_sec,
        &config.groups,
        &config.digital_inputs,
    )
}

fn device_digests(
    tags: &[String],
    includes: &[String],
    excludes: &[String],
    lamps: u32,
    interval: u32,
    groups: &[DaliGroup],
    inputs: &[DigitalInputEntry],
) -> BTreeMap<String, String> {
    [
        ("tags", digest(&tags)),
        ("includes", digest(&includes)),
        ("excludes", digest(&excludes)),
        ("daliLampCount", digest(&lamps)),
        ("daliGroups", digest(&groups)),
        ("digitalInputs", digest(&inputs)),
        ("sendIntervalSec", digest(&interval)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

// ───────────────────────────── device fragments ─────────────────────────────

fn u32_of(v: &Value) -> Option<u32> {
    v.as_u64()
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
        .and_then(|n| u32::try_from(n).ok())
}

/// `lw_configGroups` → [`DaliGroup`]s. Lenient: firmware-only fields (`stdLevel`, `dayState`,
/// `channel`, `scheduler`) are ignored, malformed entries skipped.
pub fn parse_groups(v: Option<&Value>) -> Vec<DaliGroup> {
    let Some(Value::Array(list)) = v else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|g| {
            let id = u32_of(g.get("id")?)?;
            Some(DaliGroup {
                id,
                name: g
                    .get("name")
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("Group {id}"), str::to_owned),
                converters: g
                    .get("converters")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(u32_of).collect())
                    .unwrap_or_default(),
                min_level: g.get("minLevel").and_then(u32_of).unwrap_or(0),
                max_level: g.get("maxLevel").and_then(u32_of).unwrap_or(100),
                lux_threshold: g.get("luxThreshold").and_then(u32_of),
                slave_of: g.get("slaveOf").and_then(u32_of),
                dynamic_control: g
                    .get("dynamicControl")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                automatic_switch: g
                    .get("automaticSwitch")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// `lw_configDigitalInputs` → [`DigitalInputEntry`]s (`id` → `index`, D1 `title`). Lenient.
pub fn parse_digital_inputs(v: Option<&Value>) -> Vec<DigitalInputEntry> {
    let Some(Value::Array(list)) = v else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let index = u32_of(e.get("id")?)?;
            let input_type = match e.get("type").and_then(Value::as_str)? {
                "daliAction" => DigitalInputType::DaliAction,
                "impulseCounter" => DigitalInputType::ImpulseCounter,
                "measurement" => DigitalInputType::Measurement,
                _ => return None,
            };
            let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(DigitalInputEntry {
                index,
                title: s("title").unwrap_or_default(),
                settings: DigitalInputSettings {
                    input_type,
                    is_active: e.get("isActive").and_then(Value::as_bool).unwrap_or(true),
                    init_count: e.get("initCount").and_then(u32_of),
                    increment: e.get("increment").and_then(u32_of),
                    unit: s("unit"),
                    high_level_action: s("highLevelAction"),
                    low_level_action: s("lowLevelAction"),
                    groups: e
                        .get("groups")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(u32_of).collect())
                        .unwrap_or_default(),
                },
            })
        })
        .collect()
}

// ───────────────────────────── digests / versions ─────────────────────────────

/// Stable short digest (FNV-1a 64, hex) of a value's canonical JSON. Used to detect changed
/// library items / floorplans between the applied and the bundled library (PRD §14).
pub fn digest<T: serde::Serialize>(value: &T) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Digest per selected library item (custom items are device-local and not tracked).
pub fn item_digests(library: &Library, items: &[LibraryItem]) -> BTreeMap<String, String> {
    items
        .iter()
        .filter(|i| library.item(i.id()).is_some())
        .map(|i| (i.id().to_owned(), digest(i)))
        .collect()
}

/// `MAJOR.MINOR.PATCH` (a pre-release suffix is ignored); `None` if unparsable.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    ))
}

/// `device` is older than `bundled` (both SemVer). Unparsable versions are never "older".
pub fn is_older_version(device: &str, bundled: &str) -> bool {
    match (parse_version(device), parse_version(bundled)) {
        (Some(d), Some(b)) => d < b,
        _ => false,
    }
}

/// `lw_demo` of a device needs an upgrade: it has a demo that exists in the bundled library
/// and its library version is older.
pub fn upgrade_available(library: &Library, demo: Option<&LwDemoState>) -> bool {
    demo.is_some_and(|d| {
        d.demo_id
            .as_deref()
            .is_some_and(|id| library.demo(id).is_some())
            && d.library_version
                .as_deref()
                .is_some_and(|v| is_older_version(v, &library.version))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_older_version("1.4.0", "1.10.0"));
        assert!(!is_older_version("1.10.0", "1.4.0"));
        assert!(!is_older_version("1.4.0", "1.4.0"));
        assert!(is_older_version("0.9", "1.0.0"));
        assert!(!is_older_version("garbage", "1.0.0"));
    }

    #[test]
    fn groups_parse_leniently() {
        let v = json!([
            { "id": 1, "name": "Shop", "minLevel": 0, "maxLevel": 100, "stdLevel": 100,
              "dayState": false, "channel": 1, "scheduler": [], "converters": [1, 2],
              "slaveOf": null, "luxThreshold": 4500 },
            { "name": "no id" }
        ]);
        let g = parse_groups(Some(&v));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].converters, vec![1, 2]);
        assert_eq!(g[0].slave_of, None);
        assert_eq!(g[0].lux_threshold, Some(4500));
    }

    #[test]
    fn digital_inputs_parse_id_as_index() {
        let v = json!([{ "id": 3, "isActive": true, "title": "Gas", "type": "impulseCounter",
                         "initCount": 1000, "increment": 10, "unit": "m3" }]);
        let d = parse_digital_inputs(Some(&v));
        assert_eq!(d[0].index, 3);
        assert_eq!(d[0].title, "Gas");
        assert_eq!(d[0].settings.init_count, Some(1000));
    }

    #[test]
    fn digest_is_stable_and_sensitive() {
        assert_eq!(digest(&json!({"a": 1})), digest(&json!({"a": 1})));
        assert_ne!(digest(&json!({"a": 1})), digest(&json!({"a": 2})));
    }
}
