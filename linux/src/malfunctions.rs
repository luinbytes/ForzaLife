use crate::{model::LifeSnapshot, telemetry::Telemetry};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarvationAction {
    Enable,
    Disable,
}

#[derive(Default)]
pub struct FuelStarvation {
    enabled: bool,
}

impl FuelStarvation {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn update(
        &mut self,
        life: &LifeSnapshot,
        telemetry: &Telemetry,
    ) -> Option<StarvationAction> {
        let enabled = life.fuel_liters <= 0.0
            && !life.is_usage_paused
            && telemetry.race_on
            && telemetry.race_position == 0
            && telemetry.gear > 0
            && telemetry.gear != 11;
        if self.enabled == enabled {
            return None;
        }
        self.enabled = enabled;
        Some(if enabled {
            StarvationAction::Enable
        } else {
            StarvationAction::Disable
        })
    }
}
