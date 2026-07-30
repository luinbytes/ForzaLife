use forzalife::{
    malfunctions::{FuelStarvation, StarvationAction},
    model::LifeSnapshot,
    telemetry::Telemetry,
};

fn life(fuel_liters: f32) -> LifeSnapshot {
    LifeSnapshot {
        car_ordinal: 42,
        fuel_liters,
        fuel_percent: fuel_liters / 60.0,
        odometer_m: 0.0,
        trip_m: 0.0,
        average_mpg: None,
        average_km_per_liter: None,
        oil_remaining_m: 500_000.0,
        is_usage_paused: false,
    }
}

fn telemetry(race_position: u8) -> Telemetry {
    Telemetry {
        race_on: true,
        timestamp_ms: 1_000,
        engine_max_rpm: 8_000.0,
        engine_idle_rpm: 900.0,
        current_engine_rpm: 6_000.0,
        yaw: 0.0,
        power_w: 80_000.0,
        torque_nm: 250.0,
        race_position,
        throttle: 255,
        car_ordinal: 42,
        num_cylinders: 4,
        position: [0.0; 3],
        speed_mps: 20.0,
        boost_psi: 5.0,
        fuel: 0.0,
        distance_m: 0.0,
        gear: 4,
    }
}

#[test]
fn empty_fuel_enables_throttle_restriction_only_during_free_roam() {
    let mut starvation = FuelStarvation::default();

    assert_eq!(
        starvation.update(&life(0.0), &telemetry(0)),
        Some(StarvationAction::Enable)
    );
    assert!(starvation.enabled());
    assert_eq!(
        starvation.update(&life(0.0), &telemetry(1)),
        Some(StarvationAction::Disable)
    );
    assert!(!starvation.enabled());
    assert_eq!(starvation.update(&life(10.0), &telemetry(0)), None);
}
