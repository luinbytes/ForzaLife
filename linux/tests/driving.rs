use forzalife::{
    driving::{DriveSession, DriveSnapshot, Gear},
    model::LifeSnapshot,
    telemetry::Telemetry,
};

fn telemetry(timestamp_ms: u32, speed_mps: f32, gear: u8) -> Telemetry {
    Telemetry {
        race_on: true,
        timestamp_ms,
        engine_max_rpm: 8_000.0,
        engine_idle_rpm: 900.0,
        current_engine_rpm: 7_200.0,
        yaw: 0.0,
        power_w: 0.0,
        torque_nm: 0.0,
        race_position: 2,
        throttle: 128,
        car_ordinal: 42,
        num_cylinders: 4,
        position: [0.0; 3],
        speed_mps,
        boost_psi: 0.0,
        fuel: 1.0,
        distance_m: 0.0,
        gear,
    }
}

fn life(fuel_liters: f32) -> LifeSnapshot {
    LifeSnapshot {
        car_ordinal: 42,
        fuel_liters,
        fuel_percent: fuel_liters / 60.0,
        odometer_m: 0.0,
        trip_m: 0.0,
        average_mpg: None,
        average_km_per_liter: None,
        oil_remaining_m: 0.0,
        is_usage_paused: false,
    }
}

#[test]
fn drive_snapshot_converts_speed_and_maps_gear() {
    let snapshot = DriveSnapshot::from_telemetry(&telemetry(1_000, 10.0, 4));

    assert_eq!(snapshot.speed_kmh, 36.0);
    assert!((snapshot.speed_mph - 22.369_363).abs() < 0.001);
    assert_eq!(snapshot.gear, Gear::Forward(3));
    assert_eq!(snapshot.throttle_percent, 50);
    assert_eq!(snapshot.race_position, Some(2));
    assert_eq!(snapshot.shift_stage(), 2);
}

#[test]
fn drive_snapshot_handles_reverse_neutral_and_rpm_limits() {
    let mut sample = telemetry(1_000, -2.0, 0);
    sample.current_engine_rpm = -10.0;
    assert_eq!(DriveSnapshot::from_telemetry(&sample).gear, Gear::Reverse);
    assert_eq!(DriveSnapshot::from_telemetry(&sample).rpm_percent, 0.0);

    sample.gear = 1;
    sample.current_engine_rpm = 99_000.0;
    let snapshot = DriveSnapshot::from_telemetry(&sample);
    assert_eq!(snapshot.gear, Gear::Neutral);
    assert_eq!(snapshot.rpm_percent, 1.0);
}

#[test]
fn session_tracks_distance_speed_fuel_refuels_and_reset() {
    let mut session = DriveSession::default();
    session.update(&telemetry(1_000, 10.0, 4), &life(60.0));
    session.update(&telemetry(2_000, 20.0, 4), &life(58.0));

    assert_eq!(session.elapsed_s, 1.0);
    assert_eq!(session.distance_m, 20.0);
    assert_eq!(session.average_speed_mps(), 20.0);
    assert_eq!(session.max_speed_mps, 20.0);
    assert_eq!(session.fuel_used_liters, 2.0);
    assert_eq!(session.refuels, 0);

    session.update(&telemetry(3_000, 5.0, 4), &life(60.0));
    assert_eq!(session.refuels, 1);
    assert_eq!(session.fuel_used_liters, 2.0);

    session.reset();
    assert_eq!(session.current_car, Some(42));
    assert_eq!(session.distance_m, 0.0);
    assert_eq!(session.fuel_used_liters, 0.0);
}

#[test]
fn changing_vehicle_starts_a_clean_session_without_changing_life_state() {
    let mut session = DriveSession::default();
    session.update(&telemetry(1_000, 10.0, 4), &life(60.0));

    let mut other = telemetry(2_000, 30.0, 4);
    other.car_ordinal = 99;
    session.update(&other, &life(40.0));

    assert_eq!(session.current_car, Some(99));
    assert_eq!(session.elapsed_s, 0.0);
    assert_eq!(session.distance_m, 0.0);
    assert_eq!(session.fuel_used_liters, 0.0);
}
