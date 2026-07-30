use forzalife::input_proxy::{limit_analog_throttle, limit_digital_throttle};

#[test]
fn starvation_pulse_limits_keyboard_and_trigger_without_stopping_the_car() {
    assert!(limit_digital_throttle(true, false, 400));
    assert_eq!(limit_analog_throttle(1023, 0, 1023, false, 400), 1023);

    let first_cycle: Vec<_> = [100, 550, 800, 1_300, 1_850]
        .map(|time| limit_analog_throttle(1023, 0, 1023, true, time))
        .into();
    let later_cycle: Vec<_> = [2_300, 2_750, 3_000, 3_500, 4_050]
        .map(|time| limit_analog_throttle(1023, 0, 1023, true, time))
        .into();

    assert_ne!(first_cycle, later_cycle);
    assert!(
        first_cycle
            .iter()
            .chain(&later_cycle)
            .all(|value| *value == 0 || (409..=695).contains(value))
    );
}
