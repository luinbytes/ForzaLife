use crate::telemetry::Telemetry;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io, path::Path};

pub const DEFAULT_TANK_LITERS: f32 = 60.0;
pub const OIL_INTERVAL_METERS: f32 = 500_000.0;
const LITERS_PER_IMPERIAL_GALLON: f32 = 4.546_09;
const KM_PER_MILE: f32 = 1.609_344;
const ECONOMY_WINDOW_METERS: f32 = 1_500.0;

#[derive(Clone, Debug, PartialEq)]
pub struct LifeSnapshot {
    pub car_ordinal: i32,
    pub fuel_liters: f32,
    pub fuel_percent: f32,
    pub odometer_m: f32,
    pub trip_m: f32,
    pub average_mpg: Option<f32>,
    pub average_km_per_liter: Option<f32>,
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
    #[serde(default)]
    fuel_used_liters: f32,
    #[serde(default)]
    economy_distance_m: f32,
    #[serde(default)]
    economy_window_distance_m: f32,
    #[serde(default)]
    economy_window_fuel_liters: f32,
    #[serde(default)]
    economy_initialized: bool,
    #[serde(default)]
    is_electric: bool,
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
            fuel_used_liters: 0.0,
            economy_distance_m: 0.0,
            economy_window_distance_m: 0.0,
            economy_window_fuel_liters: 0.0,
            economy_initialized: false,
            is_electric: false,
            oil_since_service_m: 0.0,
            is_usage_paused: false,
        }
    }
}

#[derive(Clone, Copy)]
struct LastSample {
    car_ordinal: i32,
    timestamp_ms: u32,
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
        self.update_with_vehicle(telemetry, tank_capacity_liters, 2020)
    }

    pub fn update_with_vehicle(
        &mut self,
        telemetry: &Telemetry,
        tank_capacity_liters: f32,
        model_year: i32,
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
            let speed_mps = telemetry.speed_mps.abs();
            if speed_mps >= 0.1 {
                distance_delta = speed_mps * elapsed_s;
            }
        }
        self.last = Some(LastSample {
            car_ordinal: telemetry.car_ordinal,
            timestamp_ms: telemetry.timestamp_ms,
        });

        let vehicle = self.vehicles.entry(telemetry.car_ordinal).or_default();
        if !vehicle.economy_initialized {
            vehicle.fuel_used_liters = 0.0;
            vehicle.economy_distance_m = 0.0;
            vehicle.economy_initialized = true;
        }
        vehicle.is_electric = telemetry.num_cylinders == 0;
        if tank_capacity_liters >= 10.0
            && (vehicle.tank_liters - tank_capacity_liters).abs() > f32::EPSILON
        {
            let fuel_percent = vehicle.fuel_liters / vehicle.tank_liters.max(1.0);
            vehicle.tank_liters = tank_capacity_liters;
            vehicle.fuel_liters = fuel_percent * tank_capacity_liters;
        }
        let mut actual_consumed_liters = 0.0;
        if !vehicle.is_usage_paused && elapsed_s > 0.0 {
            let consumed = if vehicle.is_electric {
                let traction_kw = telemetry.power_w.max(0.0) / 1_000.0;
                (traction_kw + 1.8) * elapsed_s / 3_600.0
            } else if telemetry.current_engine_rpm > 100.0 {
                combustion_liters_per_second(
                    telemetry.power_w,
                    telemetry.torque_nm,
                    telemetry.num_cylinders,
                    model_year,
                    telemetry.current_engine_rpm,
                    telemetry.engine_max_rpm,
                    telemetry.throttle,
                ) * elapsed_s
            } else {
                0.0
            };
            let before = vehicle.fuel_liters;
            vehicle.fuel_liters = (vehicle.fuel_liters - consumed).max(0.0);
            actual_consumed_liters = before - vehicle.fuel_liters;
            if !vehicle.is_electric {
                vehicle.fuel_used_liters += actual_consumed_liters;
            }
        }
        vehicle.odometer_m += distance_delta;
        vehicle.trip_m += distance_delta;
        if !vehicle.is_usage_paused && !vehicle.is_electric {
            vehicle.economy_distance_m += distance_delta;
            if distance_delta > 0.0 {
                let retained = (1.0 - distance_delta / ECONOMY_WINDOW_METERS).clamp(0.0, 1.0);
                vehicle.economy_window_distance_m =
                    vehicle.economy_window_distance_m * retained + distance_delta;
                vehicle.economy_window_fuel_liters =
                    vehicle.economy_window_fuel_liters * retained + actual_consumed_liters;
            }
        }
        vehicle.oil_since_service_m += distance_delta;

        snapshot(telemetry.car_ordinal, vehicle)
    }

    pub fn refuel(&mut self, car_ordinal: i32, liters: f32) {
        let vehicle = self.vehicles.entry(car_ordinal).or_default();
        vehicle.fuel_liters = (vehicle.fuel_liters + liters.max(0.0)).min(vehicle.tank_liters);
    }

    pub fn service_oil(&mut self, car_ordinal: i32) {
        self.vehicles
            .entry(car_ordinal)
            .or_default()
            .oil_since_service_m = 0.0;
    }

    pub fn set_odometer(&mut self, car_ordinal: i32, odometer_m: f32) {
        if odometer_m.is_finite() && odometer_m >= 0.0 {
            self.vehicles.entry(car_ordinal).or_default().odometer_m = odometer_m;
        }
    }

    pub fn set_odometer_and_save(
        &mut self,
        path: &Path,
        car_ordinal: i32,
        odometer_m: f32,
    ) -> io::Result<LifeSnapshot> {
        if !odometer_m.is_finite() || odometer_m < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "odometer must be a non-negative finite value",
            ));
        }
        let previous = self.vehicles.get(&car_ordinal).cloned();
        self.set_odometer(car_ordinal, odometer_m);
        if let Err(error) = self.save(path) {
            if let Some(previous) = previous {
                self.vehicles.insert(car_ordinal, previous);
            } else {
                self.vehicles.remove(&car_ordinal);
            }
            return Err(error);
        }
        Ok(self.current(car_ordinal).expect("saved vehicle state"))
    }

    pub fn set_fuel_percent_and_save(
        &mut self,
        path: &Path,
        car_ordinal: i32,
        fuel_percent: f32,
    ) -> io::Result<LifeSnapshot> {
        if !fuel_percent.is_finite() || !(0.0..=1.0).contains(&fuel_percent) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fuel percentage must be between zero and one",
            ));
        }
        let previous = self.vehicles.get(&car_ordinal).cloned();
        let vehicle = self.vehicles.entry(car_ordinal).or_default();
        vehicle.fuel_liters = vehicle.tank_liters * fuel_percent;
        if let Err(error) = self.save(path) {
            if let Some(previous) = previous {
                self.vehicles.insert(car_ordinal, previous);
            } else {
                self.vehicles.remove(&car_ordinal);
            }
            return Err(error);
        }
        Ok(self.current(car_ordinal).expect("saved vehicle state"))
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
    let (economy_distance_m, fuel_used_liters) = if vehicle.economy_window_distance_m >= 25.0 {
        (
            vehicle.economy_window_distance_m,
            vehicle.economy_window_fuel_liters,
        )
    } else {
        (vehicle.economy_distance_m, vehicle.fuel_used_liters)
    };
    let economy = (!vehicle.is_electric && economy_distance_m >= 25.0 && fuel_used_liters >= 0.001)
        .then(|| {
            let imperial_gallons = fuel_used_liters / LITERS_PER_IMPERIAL_GALLON;
            let kilometers = economy_distance_m / 1_000.0;
            (
                kilometers / KM_PER_MILE / imperial_gallons,
                kilometers / fuel_used_liters,
            )
        });
    LifeSnapshot {
        car_ordinal,
        fuel_liters: vehicle.fuel_liters,
        fuel_percent: vehicle.fuel_liters / vehicle.tank_liters.max(1.0),
        odometer_m: vehicle.odometer_m,
        trip_m: vehicle.trip_m,
        average_mpg: economy.map(|value| value.0),
        average_km_per_liter: economy.map(|value| value.1),
        oil_remaining_m: OIL_INTERVAL_METERS - vehicle.oil_since_service_m,
        is_usage_paused: vehicle.is_usage_paused,
    }
}

fn combustion_liters_per_second(
    power_w: f32,
    torque_nm: f32,
    cylinders: i32,
    model_year: i32,
    current_rpm: f32,
    max_rpm: f32,
    throttle: u8,
) -> f32 {
    let (estimated_displacement_liters, cylinder_bsfc_adjustment) = match cylinders {
        12.. => (8.0, 70.0),
        10..=11 => (7.0, 60.0),
        8..=9 => (5.0, 50.0),
        6..=7 => (3.5, 25.0),
        4..=5 => (2.0, 0.0),
        _ => (1.0, -20.0),
    };
    let (age_factor, year_bsfc_adjustment) = match model_year {
        2020.. => (1.0, -30.0),
        2010..=2019 => (1.05, -15.0),
        2000..=2009 => (1.1, 0.0),
        1990..=1999 => (1.2, 8.0),
        1980..=1989 => (1.3, 18.0),
        1970..=1979 => (1.4, 27.0),
        1000..=1969 => (1.45, 30.0),
        _ => (1.0, 0.0),
    };
    let idle_liters_per_second = 0.000_12 * estimated_displacement_liters * age_factor;
    let bsfc_grams_per_kwh = 230.0 + cylinder_bsfc_adjustment + year_bsfc_adjustment;
    let throttle_ratio = f32::from(throttle) / 255.0;
    let torque_power_w =
        torque_nm.max(0.0) * current_rpm.max(0.0) * std::f32::consts::TAU / 60.0 * throttle_ratio;
    let load_power_w = power_w.max(0.0).max(torque_power_w);
    let load_liters_per_second = load_power_w / 1_000.0 * bsfc_grams_per_kwh / 2_700_000.0;
    let rpm_ratio = if current_rpm.is_finite() && max_rpm.is_finite() && max_rpm > 100.0 {
        (current_rpm / max_rpm).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let high_rpm_multiplier = if rpm_ratio <= 0.5 {
        1.25
    } else if rpm_ratio <= 0.75 {
        1.25 + 0.75 * (rpm_ratio - 0.5) / 0.25
    } else if rpm_ratio <= 0.9 {
        2.0 + (rpm_ratio - 0.75) / 0.15
    } else {
        3.0 + 0.5 * (rpm_ratio - 0.9) / 0.1
    };
    (idle_liters_per_second + load_liters_per_second) * high_rpm_multiplier
}

const fn default_tank_liters() -> f32 {
    DEFAULT_TANK_LITERS
}
