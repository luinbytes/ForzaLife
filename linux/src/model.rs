use crate::telemetry::Telemetry;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io, path::Path};

pub const DEFAULT_TANK_LITERS: f32 = 60.0;
pub const OIL_INTERVAL_METERS: f32 = 500_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct LifeSnapshot {
    pub car_ordinal: i32,
    pub fuel_liters: f32,
    pub fuel_percent: f32,
    pub odometer_m: f32,
    pub trip_m: f32,
    pub oil_remaining_m: f32,
    pub is_usage_paused: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VehicleLife {
    fuel_liters: f32,
    #[serde(default = "default_tank_liters")]
    tank_liters: f32,
    odometer_m: f32,
    #[serde(default)]
    trip_m: f32,
    oil_since_service_m: f32,
    #[serde(default)]
    is_usage_paused: bool,
}

impl Default for VehicleLife {
    fn default() -> Self {
        Self {
            fuel_liters: DEFAULT_TANK_LITERS,
            tank_liters: DEFAULT_TANK_LITERS,
            odometer_m: 0.0,
            trip_m: 0.0,
            oil_since_service_m: 0.0,
            is_usage_paused: false,
        }
    }
}

#[derive(Clone, Copy)]
struct LastSample {
    car_ordinal: i32,
    timestamp_ms: u32,
    distance_m: f32,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Simulation {
    vehicles: HashMap<i32, VehicleLife>,
    #[serde(skip)]
    last: Option<LastSample>,
}

impl Simulation {
    pub fn update(&mut self, telemetry: &Telemetry) -> LifeSnapshot {
        self.update_with_capacity(telemetry, DEFAULT_TANK_LITERS)
    }

    pub fn update_with_capacity(
        &mut self,
        telemetry: &Telemetry,
        tank_capacity_liters: f32,
    ) -> LifeSnapshot {
        let mut elapsed_s = 0.0;
        let mut distance_delta = 0.0;
        if let Some(last) = self.last
            && last.car_ordinal == telemetry.car_ordinal
        {
            elapsed_s = telemetry.timestamp_ms.wrapping_sub(last.timestamp_ms) as f32 / 1_000.0;
            if !(0.0..=1.0).contains(&elapsed_s) {
                elapsed_s = 0.0;
            }
            let candidate = telemetry.distance_m - last.distance_m;
            if (0.0..=200.0).contains(&candidate) {
                distance_delta = candidate;
            }
        }
        self.last = Some(LastSample {
            car_ordinal: telemetry.car_ordinal,
            timestamp_ms: telemetry.timestamp_ms,
            distance_m: telemetry.distance_m,
        });

        let vehicle = self.vehicles.entry(telemetry.car_ordinal).or_default();
        if tank_capacity_liters >= 10.0
            && (vehicle.tank_liters - tank_capacity_liters).abs() > f32::EPSILON
        {
            let fuel_percent = vehicle.fuel_liters / vehicle.tank_liters.max(1.0);
            vehicle.tank_liters = tank_capacity_liters;
            vehicle.fuel_liters = fuel_percent * tank_capacity_liters;
        }
        if !vehicle.is_usage_paused && elapsed_s > 0.0 && telemetry.current_engine_rpm > 100.0 {
            let rpm_ratio =
                (telemetry.current_engine_rpm / telemetry.engine_max_rpm.max(1.0)).clamp(0.0, 1.5);
            let throttle = f32::from(telemetry.throttle) / 255.0;
            let idle_lph = 0.7 + 0.8 * rpm_ratio * (0.25 + 0.75 * throttle);
            let load_lph = telemetry.power_w.max(0.0) / 1_000.0 * 0.32;
            vehicle.fuel_liters =
                (vehicle.fuel_liters - (idle_lph + load_lph) * elapsed_s / 3_600.0).max(0.0);
        }
        vehicle.odometer_m += distance_delta;
        vehicle.trip_m += distance_delta;
        vehicle.oil_since_service_m += distance_delta;

        snapshot(telemetry.car_ordinal, vehicle)
    }

    pub fn refuel(&mut self, car_ordinal: i32) {
        let vehicle = self.vehicles.entry(car_ordinal).or_default();
        vehicle.fuel_liters = vehicle.tank_liters;
    }

    pub fn service_oil(&mut self, car_ordinal: i32) {
        self.vehicles
            .entry(car_ordinal)
            .or_default()
            .oil_since_service_m = 0.0;
    }

    pub fn toggle_usage(&mut self, car_ordinal: i32) -> bool {
        let vehicle = self.vehicles.entry(car_ordinal).or_default();
        vehicle.is_usage_paused = !vehicle.is_usage_paused;
        vehicle.is_usage_paused
    }

    pub fn current(&self, car_ordinal: i32) -> Option<LifeSnapshot> {
        self.vehicles
            .get(&car_ordinal)
            .map(|vehicle| snapshot(car_ordinal, vehicle))
    }

    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temporary, path)
    }
}

fn snapshot(car_ordinal: i32, vehicle: &VehicleLife) -> LifeSnapshot {
    LifeSnapshot {
        car_ordinal,
        fuel_liters: vehicle.fuel_liters,
        fuel_percent: vehicle.fuel_liters / vehicle.tank_liters.max(1.0),
        odometer_m: vehicle.odometer_m,
        trip_m: vehicle.trip_m,
        oil_remaining_m: (OIL_INTERVAL_METERS - vehicle.oil_since_service_m).max(0.0),
        is_usage_paused: vehicle.is_usage_paused,
    }
}

const fn default_tank_liters() -> f32 {
    DEFAULT_TANK_LITERS
}
