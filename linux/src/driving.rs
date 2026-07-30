use crate::{model::LifeSnapshot, telemetry::Telemetry};

const KMH_PER_MPS: f32 = 3.6;
const MPH_PER_MPS: f32 = 2.236_936_3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gear {
    Reverse,
    Neutral,
    Forward(u8),
}

impl Gear {
    pub fn label(self) -> String {
        match self {
            Self::Reverse => "R".to_owned(),
            Self::Neutral => "N".to_owned(),
            Self::Forward(gear) => gear.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveSnapshot {
    pub speed_kmh: f32,
    pub speed_mph: f32,
    pub rpm: f32,
    pub rpm_percent: f32,
    pub gear: Gear,
    pub throttle_percent: u8,
    pub race_on: bool,
    pub race_position: Option<u8>,
}

impl DriveSnapshot {
    pub fn from_telemetry(telemetry: &Telemetry) -> Self {
        let max_rpm = telemetry.engine_max_rpm.max(1.0);
        Self {
            speed_kmh: telemetry.speed_mps.abs() * KMH_PER_MPS,
            speed_mph: telemetry.speed_mps.abs() * MPH_PER_MPS,
            rpm: telemetry.current_engine_rpm.max(0.0),
            rpm_percent: (telemetry.current_engine_rpm / max_rpm).clamp(0.0, 1.0),
            gear: match telemetry.gear {
                0 => Gear::Reverse,
                1 => Gear::Neutral,
                gear => Gear::Forward(gear - 1),
            },
            throttle_percent: ((u16::from(telemetry.throttle) * 100) / 255) as u8,
            race_on: telemetry.race_on,
            race_position: (telemetry.race_on && telemetry.race_position > 0)
                .then_some(telemetry.race_position),
        }
    }

    pub fn shift_stage(self) -> u8 {
        match self.rpm_percent {
            value if value >= 0.95 => 3,
            value if value >= 0.85 => 2,
            value if value >= 0.70 => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DriveSession {
    pub current_car: Option<i32>,
    pub elapsed_s: f32,
    pub distance_m: f32,
    pub max_speed_mps: f32,
    pub fuel_used_liters: f32,
    pub refuels: u32,
    previous_timestamp_ms: Option<u32>,
    previous_fuel_liters: Option<f32>,
}

impl DriveSession {
    pub fn update(&mut self, telemetry: &Telemetry, life: &LifeSnapshot) {
        if self.current_car != Some(telemetry.car_ordinal) {
            *self = Self {
                current_car: Some(telemetry.car_ordinal),
                ..Self::default()
            };
        }

        let elapsed_s = self
            .previous_timestamp_ms
            .map(|previous| telemetry.timestamp_ms.wrapping_sub(previous) as f32 / 1_000.0)
            .filter(|elapsed| (0.0..=1.0).contains(elapsed))
            .unwrap_or_default();
        self.elapsed_s += elapsed_s;
        self.distance_m += telemetry.speed_mps.abs() * elapsed_s;
        self.max_speed_mps = self.max_speed_mps.max(telemetry.speed_mps.abs());
        if let Some(previous_fuel) = self.previous_fuel_liters {
            if life.fuel_liters > previous_fuel + f32::EPSILON {
                self.refuels += 1;
            } else {
                self.fuel_used_liters += (previous_fuel - life.fuel_liters).max(0.0);
            }
        }
        self.previous_timestamp_ms = Some(telemetry.timestamp_ms);
        self.previous_fuel_liters = Some(life.fuel_liters);
    }

    pub fn reset(&mut self) {
        let car = self.current_car;
        *self = Self::default();
        self.current_car = car;
    }

    pub fn average_speed_mps(self) -> f32 {
        if self.elapsed_s > 0.0 {
            self.distance_m / self.elapsed_s
        } else {
            0.0
        }
    }
}
