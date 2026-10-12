//! Faults (PRD §7.2, §7.4, §7.5; DECISIONS D8, D9).
//!
//! - Fire: `POST` the alarm (firmware text + severity from `payload::alarm`); firing an open
//!   fault again re-posts it (Cumulocity counts up).
//!   - Slave timeout: also stop that slave's measurements and send the `lw_modbus` event.
//!   - DALI lamp: lamp alarms set `Lamp Status` 0, control-gear / converter failures set
//!     `Converter Status` 0, `lost` / `notExisting` stop the lamp's measurements (firmware §1.2).
//! - The effect lasts until the alarm is CLEARED in Cumulocity: by the presenter (realtime
//!   `/alarms/<id>`, or the open-alarm list after a reconnect), by "Clear all", or by a scenario
//!   step `active: false` (which `PUT`s CLEARED itself). ACKNOWLEDGED only changes the status.
//! - End of an effect: `lw_alarm` "Alarm cleared: <type>" (the firmware's recovery), then
//!   resume / restore. Derived alarms (bus cascade) are recovered by the simulator: event, plus
//!   CLEARED unless realistic mode.

use std::collections::BTreeSet;

use serde_json::Value;
use tokio::time::Instant;

use super::core::{AlarmTrack, RECENT_CLEARED, RestoredAlarm, SessionCore};
use crate::device::setup;
use crate::error::AppResult;
use crate::events::FaultChanged;
use crate::library::Library;
use crate::model::{
    ActiveFault, FaultDefinition, FaultTarget, LibraryItem, LwDemoState, OpenAlarmStatus,
};
use crate::payload::alarm::{
    AlarmSpec, FaultSetup, SENSOR_BRIGHTNESS, SENSOR_MOTION, bus_power_cascade,
    control_gear_failure_spec, fault_catalog, is_bus_power_alarm, resolve_fault,
};
use crate::payload::event::build_modbus_timeout_event;

use super::emit::SessionEvent;
use super::outbound::OutboundPayload;

/// Number of DALI lamps of a device: `daliLampCount`, or more when an item addresses a higher
/// lamp (such a lamp exists too), at most 64. The session, the fault catalog and the fault
/// resolution all use this one rule.
pub fn lamp_count(dali_lamp_count: u32, items: &[&LibraryItem]) -> u32 {
    let highest = items
        .iter()
        .filter(|i| matches!(i, LibraryItem::Dali { .. }))
        .filter_map(|i| i.index())
        .max()
        .unwrap_or(0);
    dali_lamp_count.max(highest).min(64)
}

/// All faults selectable for a device setup: DALI lamps and bus, per slave the timeout fault
/// and **every** custom alarm (`FaultTarget::SlaveAlarm`), and each device-level `customAlarm`
/// item (`FaultTarget::CustomAlarm`, type `lw_demo_<name>`). Slave faults use the persisted
/// `lw_demo.slaves` ids; text/severity come from the firmware catalog or the library.
pub fn catalog(library: &Library, state: &LwDemoState) -> Vec<FaultDefinition> {
    let Ok(resolved) = setup::resolve(library, state) else {
        return Vec::new();
    };
    let slaves = resolved.slave_refs();
    let items = resolved.item_refs();
    fault_catalog(&FaultSetup {
        lamp_count: lamp_count(state.dali_lamp_count, &items),
        slaves: &slaves,
        items: &items,
    })
}

/// Lamp status flags implied by the active faults of lamp `n`.
fn lamp_flags(faults: &[ActiveFault], n: u32) -> (bool, bool, bool) {
    let (mut converter_ok, mut lamp_ok, mut lost) = (true, true, false);
    for f in faults {
        if let FaultTarget::DaliLamp { lamp, alarm_type } = &f.target
            && *lamp == n
        {
            if alarm_type.starts_with("lw_lampFailure_") {
                lamp_ok = false;
            } else if alarm_type.starts_with("lw_controlGearFailure_")
                || alarm_type.starts_with("lw_converterFailure_")
            {
                converter_ok = false;
            } else if alarm_type.ends_with("_lost") || alarm_type.ends_with("_notExisting") {
                lost = true;
            }
        }
    }
    (converter_ok, lamp_ok, lost)
}

impl SessionCore {
    fn fault_spec(&self, target: &FaultTarget) -> AppResult<AlarmSpec> {
        let slaves: Vec<_> = self.slaves.iter().map(|(d, a)| (d, a)).collect();
        let items: Vec<&LibraryItem> = self.items.iter().map(|i| &i.def).collect();
        let lamp_count = self.lamps.keys().max().copied().unwrap_or(0);
        resolve_fault(
            target,
            &FaultSetup {
                lamp_count,
                slaves: &slaves,
                items: &items,
            },
        )
    }

    pub(super) fn fault_event(&mut self, target: FaultTarget, fault: Option<ActiveFault>) {
        let revision = self.bump();
        self.events.push(SessionEvent::Fault(FaultChanged {
            revision,
            device_id: self.device_id.clone(),
            target,
            fault,
        }));
        self.dirty = true;
    }

    fn update_lamp(&mut self, n: u32) {
        let (converter_ok, lamp_ok, lost) = lamp_flags(&self.faults, n);
        let bus_lost = self.bus_down() && self.lamp_grouped(n);
        if let Some(l) = self.lamps.get_mut(&n) {
            l.converter_ok = converter_ok;
            l.lamp_ok = lamp_ok;
            l.lost = lost || bus_lost;
        }
        self.send_lamp(n);
    }

    /// A DALI bus power fault is active: no gear or sensor answers.
    pub(super) fn bus_down(&self) -> bool {
        self.faults.iter().any(|f| {
            matches!(&f.target, FaultTarget::DaliBus { alarm_type } if is_bus_power_alarm(alarm_type))
        })
    }

    /// The alarms a bus power failure raises on this setup (grouped lamps, brightness / motion
    /// sensor items), whether or not the bus is down.
    fn bus_cascade_specs(&self) -> Vec<AlarmSpec> {
        let lamps: Vec<u32> = self
            .lamps
            .keys()
            .copied()
            .filter(|&n| self.lamp_grouped(n))
            .collect();
        let sensors: Vec<(&str, u32)> = self
            .items
            .iter()
            .filter_map(|i| match &i.def {
                LibraryItem::DaliBrightness { index, .. } => Some((SENSOR_BRIGHTNESS, *index)),
                LibraryItem::DaliMotion { index, .. } => Some((SENSOR_MOTION, *index)),
                _ => None,
            })
            .collect();
        bus_power_cascade(&lamps, &sensors)
    }

    /// Bring the derived alarms in line with the faults (firmware §2.1): while a bus power fault
    /// is active every grouped lamp is lost (`lw_converter_1_NN_lost` +
    /// `lw_controlGearFailure_1_NN`, no measurements) and every brightness / motion sensor too
    /// (`_lost`); a lamp's own `_lost` fault also holds its `lw_controlGearFailure_1_NN`
    /// (`LSY_IsGearGenericFailureActive`). When the last cause goes, they recover ("Alarm
    /// cleared" event, CLEARED unless realistic mode). An alarm that is already open (a lamp fault
    /// of its own, a restored alarm) is not raised twice, and one a fault of its own still holds
    /// is not cleared. One the presenter cleared in Cumulocity while the bus is down stays
    /// cleared (`suppressed`, D9): the lamp stays lost, and recovers with the bus (event only).
    /// All `_lost` alarms are queued before the gear failures, so the floorplan lamps turn red
    /// (and green) as early as possible. An alarm whose POST failed is raised again by the next
    /// sync (bus fault change, group change, session start). Idempotent.
    pub(super) fn sync_bus_cascade(&mut self) {
        let mut wanted = if self.bus_down() {
            self.bus_cascade_specs()
        } else {
            Vec::new()
        };
        for f in &self.faults {
            if let FaultTarget::DaliLamp { lamp, alarm_type } = &f.target
                && alarm_type.ends_with("_lost")
            {
                let spec = control_gear_failure_spec(*lamp);
                if !wanted.iter().any(|w| w.alarm_type == spec.alarm_type) {
                    wanted.push(spec);
                }
            }
        }
        wanted.sort_by_key(|s| !s.alarm_type.ends_with("_lost"));
        let mut changed: Vec<String> = Vec::new();
        for spec in &wanted {
            let new = self.bus_cascade.insert(spec.alarm_type.clone());
            let track = self.alarms.get(&spec.alarm_type);
            if !track.is_some_and(|t| t.active || t.suppressed) {
                self.raise_alarm(spec);
                changed.push(spec.alarm_type.clone());
            } else if new {
                changed.push(spec.alarm_type.clone());
            }
        }
        let mut gone: Vec<String> = self
            .bus_cascade
            .iter()
            .filter(|t| !wanted.iter().any(|w| w.alarm_type == **t))
            .cloned()
            .collect();
        gone.sort_by_key(|t| !t.ends_with("_lost"));
        let realistic = self.demo.realistic_mode;
        for t in gone {
            changed.push(t.clone());
            self.bus_cascade.remove(&t);
            if self.faults.iter().any(|f| f.alarm_type == t) {
                continue; // a fault of its own still holds it
            }
            // An alarm that was never created (failed POST) has nothing to recover; one cleared
            // in Cumulocity meanwhile gets the event only.
            let (active, suppressed) = self
                .alarms
                .get(&t)
                .map_or((false, false), |a| (a.active, a.suppressed));
            if active || suppressed {
                self.recover_alarm(&t, active && !realistic);
            }
        }
        if changed.is_empty() {
            return;
        }
        // Lamps and sensors whose alarms changed pick up the new state (lost, measurements).
        let lamps: BTreeSet<u32> = changed.iter().filter_map(|t| lamp_of_alarm(t)).collect();
        for n in lamps {
            if self.lamps.contains_key(&n) {
                self.update_lamp(n);
            }
        }
        if changed
            .iter()
            .any(|t| t.starts_with("lw_brightness_") || t.starts_with("lw_motion_"))
        {
            for idx in 0..self.items.len() {
                if matches!(
                    self.items[idx].def,
                    LibraryItem::DaliBrightness { .. } | LibraryItem::DaliMotion { .. }
                ) {
                    self.send_items.insert(idx);
                }
            }
        }
        self.dirty = true;
        self.emit_changed = true;
    }

    /// Slave key → (definition, assignment) side effects on raise / clear.
    fn slave_timeout(&mut self, key: &str, active: bool) {
        let Some((def, a)) = self.slaves.iter().find(|(d, _)| d.key == key).cloned() else {
            return;
        };
        if active {
            self.timed_out.insert(key.to_owned());
            let range = a
                .registers
                .iter()
                .map(|r| r.address)
                .min()
                .zip(a.registers.iter().map(|r| r.address).max());
            let body = build_modbus_timeout_event(
                &self.device_id,
                &crate::c8y::now_iso(),
                a.id,
                a.address,
                &def.name,
                range,
            );
            self.push(OutboundPayload::Event(body), super::core::Purpose::Event);
        } else {
            self.timed_out.remove(key);
            for idx in 0..self.items.len() {
                if self.items[idx].def.slave_key() == Some(key) {
                    self.send_items.insert(idx);
                }
            }
        }
        self.dirty = true;
    }

    /// Fire (`active`) a fault, or clear it in Cumulocity (`!active`, a scenario step: `PUT`
    /// CLEARED, regardless of realistic mode, and the effect ends like a cloud clear). Firing an
    /// active fault re-posts its alarm (Cumulocity counts up); clearing an inactive one is a
    /// no-op.
    pub fn set_fault(&mut self, target: &FaultTarget, active: bool) -> AppResult<()> {
        let spec = self.fault_spec(target)?;
        let existing = self.faults.iter().position(|f| f.target == *target);
        match (active, existing) {
            (true, None) => {
                // The alarm may be open already (held by a bus failure): take it over.
                let held = self
                    .alarms
                    .get(&spec.alarm_type)
                    .filter(|t| t.active)
                    .map(|t| (t.id.clone(), t.status));
                let fault = ActiveFault {
                    target: target.clone(),
                    alarm_type: spec.alarm_type.clone(),
                    alarm_id: held.as_ref().and_then(|(id, _)| id.clone()),
                    raised_at: crate::c8y::now_iso(),
                    status: held.as_ref().map_or(OpenAlarmStatus::Active, |(_, s)| *s),
                };
                self.faults.push(fault.clone());
                if held.is_none() {
                    self.raise_alarm(&spec);
                }
                match target {
                    FaultTarget::SlaveTimeout { slave_key } => self.slave_timeout(slave_key, true),
                    FaultTarget::DaliLamp { lamp, .. } => self.update_lamp(*lamp),
                    _ => {}
                }
                self.fault_event(target.clone(), Some(fault));
                self.emit_changed = true;
            }
            (true, Some(_)) => self.raise_alarm(&spec),
            (false, Some(pos)) => {
                self.put_alarm_clear(&spec.alarm_type);
                self.end_fault(pos);
                return Ok(());
            }
            (false, None) => {}
        }
        // A lamp's own `_lost` fault holds its control gear failure as well.
        if matches!(
            target,
            FaultTarget::DaliBus { .. } | FaultTarget::DaliLamp { .. }
        ) {
            self.sync_bus_cascade();
        }
        Ok(())
    }

    /// The fault at `pos` is over (its alarm was cleared in Cumulocity): undo its effect. The
    /// firmware's "Alarm cleared" event is sent unless a bus failure still holds the alarm (then
    /// the lamp stays lost, the alarm stays cleared, and the event follows with the bus
    /// recovery).
    pub(super) fn end_fault(&mut self, pos: usize) {
        let fault = self.faults.remove(pos);
        let alarm_type = fault.alarm_type.clone();
        if self.bus_cascade.contains(&alarm_type) {
            if let Some(t) = self.alarms.get_mut(&alarm_type) {
                t.active = false;
                t.suppressed = true;
            }
        } else if self
            .alarms
            .get(&alarm_type)
            .is_some_and(|t| t.active || t.suppressed)
        {
            self.recover_alarm(&alarm_type, false);
        }
        match &fault.target {
            FaultTarget::SlaveTimeout { slave_key } => self.slave_timeout(slave_key, false),
            FaultTarget::DaliLamp { lamp, .. } => self.update_lamp(*lamp),
            FaultTarget::Sensor {
                sensor_kind, index, ..
            } => {
                // The firmware sends the sensor again once it is back.
                self.resend_sensor(sensor_kind, u32::from(*index));
            }
            _ => {}
        }
        self.fault_event(fault.target.clone(), None);
        self.emit_changed = true;
        if matches!(
            fault.target,
            FaultTarget::DaliBus { .. } | FaultTarget::DaliLamp { .. }
        ) {
            self.sync_bus_cascade();
        }
    }

    /// Alarm `id` is CLEARED in Cumulocity (realtime, a missing entry in the open-alarm list, or
    /// the echo of our own clear). A fault on it ends; a cascade or effect alarm is marked
    /// cleared and not raised again while its cause lasts. Unknown / already inactive ids are
    /// ignored (the echo of a clear the session made itself).
    pub fn alarm_cleared_in_cloud(&mut self, id: &str) {
        if !self.recent_cleared.iter().any(|c| c == id) {
            if self.recent_cleared.len() >= RECENT_CLEARED {
                self.recent_cleared.pop_front();
            }
            self.recent_cleared.push_back(id.to_owned());
        }
        let Some(alarm_type) = self
            .alarms
            .iter()
            .find(|(_, t)| t.active && t.id.as_deref() == Some(id))
            .map(|(k, _)| k.clone())
        else {
            return;
        };
        // A create is in flight (the alarm was fired again): the presenter may have cleared the
        // old alarm just before it, and the answer says whether the new one is the same id.
        if self.alarms.get(&alarm_type).is_some_and(|t| t.inflight > 0) {
            return;
        }
        if let Some(pos) = self.faults.iter().position(|f| f.alarm_type == alarm_type) {
            self.end_fault(pos);
            return;
        }
        if let Some(t) = self.alarms.get_mut(&alarm_type) {
            t.active = false;
            t.suppressed = true;
            t.clear_pending = false;
        }
        self.dirty = true;
    }

    /// Alarm `id` is open in Cumulocity with `status` (acknowledged, or back to active): the
    /// fault on it shows the new status; the effect stays.
    pub fn alarm_status_in_cloud(&mut self, id: &str, status: OpenAlarmStatus) {
        let Some((alarm_type, track)) = self
            .alarms
            .iter_mut()
            .find(|(_, t)| t.active && t.id.as_deref() == Some(id))
        else {
            // Maybe the create answer that tells the id is still on its way.
            if self.recent_status.len() >= RECENT_CLEARED {
                self.recent_status.pop_front();
            }
            self.recent_status.push_back((id.to_owned(), status));
            return;
        };
        track.status = status;
        let alarm_type = alarm_type.clone();
        let changed: Vec<ActiveFault> = self
            .faults
            .iter_mut()
            .filter(|f| f.alarm_type == alarm_type && f.status != status)
            .map(|f| {
                f.status = status;
                f.alarm_id = Some(id.to_owned());
                f.clone()
            })
            .collect();
        for f in changed {
            self.fault_event(f.target.clone(), Some(f));
        }
    }

    /// A realtime alarm notification (`id`, `status`) of the device.
    pub fn on_alarm(&mut self, alarm: &Value) {
        let Ok(id) = crate::c8y::id_of(alarm) else {
            return;
        };
        match alarm.get("status").and_then(Value::as_str) {
            Some("CLEARED") => self.alarm_cleared_in_cloud(&id),
            Some(s) => {
                if let Some(status) = OpenAlarmStatus::parse(s) {
                    self.alarm_status_in_cloud(&id, status);
                }
            }
            None => {}
        }
    }

    /// The device's open alarms as fetched at `fetched_at` (after a realtime (re)connect): an
    /// alarm the session knew before that instant and that is not in the list was cleared while
    /// nobody listened. Statuses are taken over. Alarms the session does not track are ignored.
    pub fn reconcile_open_alarms(&mut self, open: &[Value], fetched_at: Instant) {
        let open: Vec<(String, Option<OpenAlarmStatus>)> = open
            .iter()
            .filter_map(|a| {
                let id = crate::c8y::id_of(a).ok()?;
                let st = a
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(OpenAlarmStatus::parse);
                Some((id, st))
            })
            .collect();
        let gone: Vec<String> = self
            .alarms
            .values()
            .filter(|t| t.active && t.inflight == 0 && t.id_at.is_some_and(|at| at <= fetched_at))
            .filter_map(|t| t.id.clone())
            .filter(|id| !open.iter().any(|(o, _)| o == id))
            .collect();
        for id in gone {
            self.alarm_cleared_in_cloud(&id);
        }
        for (id, st) in open {
            if let Some(st) = st {
                self.alarm_status_in_cloud(&id, st);
            }
        }
    }

    /// "Clear all" (and the start scenario applied by hand): every effect ends, every alarm the
    /// session tracks counts as cleared (an effect alarm whose value is still above its
    /// threshold is not raised again until it went below), and every open alarm of the device
    /// is cleared in Cumulocity in one go (`OutboundPayload::ClearAlarms`, after everything
    /// queued before it).
    pub fn clear_all_alarms(&mut self) {
        // What ends here is remembered: when the clear fails, the alarms still open come back.
        let raised: std::collections::BTreeMap<String, String> = self
            .faults
            .iter()
            .map(|f| (f.alarm_type.clone(), f.raised_at.clone()))
            .collect();
        self.cleared_by_all = self
            .alarms
            .iter()
            .filter(|(_, t)| t.active && t.id.is_some())
            .map(|(k, t)| RestoredAlarm {
                alarm_type: k.clone(),
                id: t.id.clone(),
                raised_at: raised.get(k).cloned().unwrap_or_else(crate::c8y::now_iso),
                status: t.status,
            })
            .collect();
        for t in self.alarms.values_mut() {
            if t.active {
                t.active = false;
                t.suppressed = true;
            }
            t.clear_pending = false;
        }
        while !self.faults.is_empty() {
            self.end_fault(0);
        }
        self.sync_bus_cascade();
        self.push(
            super::outbound::OutboundPayload::ClearAlarms,
            super::core::Purpose::ClearAll,
        );
        self.emit_changed = true;
    }

    /// Alarms still open in Cumulocity when the session starts: the fault menu shows them as
    /// active (with their status), their lamp / slave side effects apply again, and
    /// alarm effects of items know that their alarm is raised. Alarms of a type this setup does
    /// not know are not tracked.
    pub(super) fn restore_alarms(&mut self, alarms: &[RestoredAlarm], now: Instant) {
        if alarms.is_empty() {
            return;
        }
        let slaves: Vec<_> = self.slaves.iter().map(|(d, a)| (d, a)).collect();
        let items: Vec<&LibraryItem> = self.items.iter().map(|i| &i.def).collect();
        let defs = fault_catalog(&FaultSetup {
            lamp_count: self.lamps.keys().max().copied().unwrap_or(0),
            slaves: &slaves,
            items: &items,
        });
        // Lamp / sensor lost alarms that are open next to an open bus power alarm are the bus
        // failure's (they recover with the bus), not faults of their own.
        let bus_restored = alarms.iter().any(|a| {
            is_bus_power_alarm(&a.alarm_type) && defs.iter().any(|d| d.alarm_type == a.alarm_type)
        });
        let cascade: Vec<String> = if bus_restored {
            self.bus_cascade_specs()
                .into_iter()
                .map(|s| s.alarm_type)
                .collect()
        } else {
            Vec::new()
        };
        for a in alarms {
            if self.alarms.get(&a.alarm_type).is_some_and(|t| t.active) {
                continue;
            }
            let track = AlarmTrack {
                id: a.id.clone(),
                active: true,
                id_at: a.id.as_ref().map(|_| now),
                status: a.status,
                ..AlarmTrack::default()
            };
            if cascade.contains(&a.alarm_type) {
                self.alarms.insert(a.alarm_type.clone(), track);
                continue;
            }
            let item_alarm = self
                .items
                .iter()
                .any(|i| item_alarm_types(&i.def).contains(&a.alarm_type));
            let def = defs.iter().find(|d| d.alarm_type == a.alarm_type);
            if def.is_none() && !item_alarm {
                continue;
            }
            self.alarms.insert(a.alarm_type.clone(), track);
            let Some(def) = def else { continue };
            let target = def.target.clone();
            if self.faults.iter().any(|f| f.target == target) {
                continue;
            }
            self.faults.push(ActiveFault {
                target: target.clone(),
                alarm_type: a.alarm_type.clone(),
                alarm_id: a.id.clone(),
                raised_at: a.raised_at.clone(),
                status: a.status,
            });
            match &target {
                FaultTarget::SlaveTimeout { slave_key } => {
                    self.timed_out.insert(slave_key.clone());
                }
                FaultTarget::DaliLamp { lamp, .. } => self.update_lamp(*lamp),
                _ => {}
            }
        }
        self.sync_bus_cascade();
    }

    /// Queue the measurements of the brightness / motion sensor `index` again.
    fn resend_sensor(&mut self, kind: &str, index: u32) {
        for idx in 0..self.items.len() {
            let hit = match &self.items[idx].def {
                LibraryItem::DaliBrightness { index: i, .. } => kind == "brightness" && *i == index,
                LibraryItem::DaliMotion { index: i, .. } => kind == "motion" && *i == index,
                _ => false,
            };
            if hit {
                self.send_items.insert(idx);
            }
        }
    }

    /// Whether a slave (by Modbus address) is in timeout.
    pub(super) fn slave_address_timed_out(&self, address: u32) -> bool {
        self.slaves
            .iter()
            .any(|(d, a)| a.address == address && self.timed_out.contains(&d.key))
    }
}

/// Lamp number of a `lw_converter_1_NN_lost` / `lw_controlGearFailure_1_NN` alarm type.
fn lamp_of_alarm(alarm_type: &str) -> Option<u32> {
    let rest = alarm_type
        .strip_prefix("lw_converter_1_")
        .or_else(|| alarm_type.strip_prefix("lw_controlGearFailure_1_"))?;
    rest.get(..2)?.parse().ok()
}

/// Alarm types the effects of an item raise.
fn item_alarm_types(item: &LibraryItem) -> Vec<String> {
    item.effects()
        .iter()
        .filter_map(|e| match e {
            crate::model::ItemEffect::Alarm { alarm_type, .. } => Some(alarm_type.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamp_count_follows_the_highest_lamp_item() {
        assert_eq!(lamp_count(4, &[]), 4);
        assert_eq!(lamp_count(70, &[]), 64);
    }

    #[test]
    fn lamp_flags_follow_alarm_family() {
        let f = |t: &str| ActiveFault {
            target: FaultTarget::DaliLamp {
                lamp: 2,
                alarm_type: t.into(),
            },
            alarm_type: t.into(),
            alarm_id: None,
            raised_at: String::new(),
            status: OpenAlarmStatus::Active,
        };
        assert_eq!(lamp_flags(&[], 2), (true, true, false));
        assert_eq!(
            lamp_flags(&[f("lw_lampFailure_1_02_openCircuit")], 2),
            (true, false, false)
        );
        assert_eq!(
            lamp_flags(&[f("lw_controlGearFailure_1_02")], 2),
            (false, true, false)
        );
        assert_eq!(
            lamp_flags(&[f("lw_converter_1_02_lost")], 2),
            (true, true, true)
        );
        assert_eq!(
            lamp_flags(&[f("lw_converter_1_02_lost")], 3),
            (true, true, false)
        );
    }
}
