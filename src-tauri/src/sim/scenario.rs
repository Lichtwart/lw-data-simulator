//! Scenario application (PRD §8): a scenario is a list of `{itemId, value | generator | fault}`
//! steps applied in order in one loop iteration (one batched send). A failing step does not stop
//! the others; the first error is returned after all steps ran.
//!
//! Faults (D9): `active: true` fires the alarm, `active: false` clears it in Cumulocity (the
//! effect ends like after a cloud clear). The start scenario applied by hand first clears every
//! open alarm of the device ("Clear all"); at session start it runs its steps only, so open
//! alarms it does not mention stay restored.

use tokio::time::Instant;

use super::core::{SessionCore, Source};
use crate::error::{AppError, AppResult};
use crate::model::ScenarioStep;

impl SessionCore {
    /// Apply steps; `Err` lists the steps that failed.
    pub(super) fn apply_scenario_steps(
        &mut self,
        steps: &[ScenarioStep],
        _now: Instant,
    ) -> AppResult<()> {
        let mut errors = Vec::new();
        for step in steps {
            let res = match step {
                ScenarioStep::Value { item_id, value } => {
                    self.set_checked(item_id, *value, Source::Scenario)
                }
                ScenarioStep::Generator { item_id, generator } => {
                    self.set_generator(item_id, Some(generator.clone()))
                }
                ScenarioStep::Fault { target, active } => self.set_fault(target, *active),
            };
            if let Err(e) = res {
                errors.push(e.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(AppError::InvalidInput(format!(
                "scenario steps failed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Apply a scenario of the device's demo by id.
    pub fn apply_scenario(&mut self, scenario_id: &str, now: Instant) -> AppResult<()> {
        let sc = self
            .scenarios
            .iter()
            .find(|s| s.id == scenario_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("scenario {scenario_id}")))?;
        if sc.start {
            self.clear_all_alarms();
        }
        self.apply_scenario_steps(&sc.set, now)
    }
}
