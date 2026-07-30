use forzalife::telemetry::{PACKET_SIZE, ParseError, parse};

fn put_i32(packet: &mut [u8; PACKET_SIZE], offset: usize, value: i32) {
    packet[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(packet: &mut [u8; PACKET_SIZE], offset: usize, value: u32) {
    packet[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(packet: &mut [u8; PACKET_SIZE], offset: usize, value: f32) {
    packet[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn parses_the_documented_fh6_packet_layout() {
    let mut packet = [0_u8; PACKET_SIZE];
    put_i32(&mut packet, 0, 1);
    put_u32(&mut packet, 4, 42_123);
    put_f32(&mut packet, 8, 8_500.0);
    put_f32(&mut packet, 12, 900.0);
    put_f32(&mut packet, 16, 4_250.0);
    put_f32(&mut packet, 56, 1.25);
    put_i32(&mut packet, 212, 1337);
    put_i32(&mut packet, 228, 6);
    put_f32(&mut packet, 244, 12.5);
    put_f32(&mut packet, 248, -3.0);
    put_f32(&mut packet, 252, 99.25);
    put_f32(&mut packet, 256, 27.0);
    put_f32(&mut packet, 260, 123_000.0);
    put_f32(&mut packet, 264, 420.0);
    put_f32(&mut packet, 284, 8.25);
    put_f32(&mut packet, 288, 0.75);
    put_f32(&mut packet, 292, 12_345.0);
    packet[314] = 3;
    packet[315] = 200;
    packet[319] = 4;

    let parsed = parse(&packet).expect("documented packet should parse");

    assert!(parsed.race_on);
    assert_eq!(parsed.timestamp_ms, 42_123);
    assert_eq!(parsed.engine_max_rpm, 8_500.0);
    assert_eq!(parsed.engine_idle_rpm, 900.0);
    assert_eq!(parsed.current_engine_rpm, 4_250.0);
    assert_eq!(parsed.yaw, 1.25);
    assert_eq!(parsed.power_w, 123_000.0);
    assert_eq!(parsed.torque_nm, 420.0);
    assert_eq!(parsed.race_position, 3);
    assert_eq!(parsed.throttle, 200);
    assert_eq!(parsed.car_ordinal, 1337);
    assert_eq!(parsed.num_cylinders, 6);
    assert_eq!(parsed.position, [12.5, -3.0, 99.25]);
    assert_eq!(parsed.speed_mps, 27.0);
    assert_eq!(parsed.boost_psi, 8.25);
    assert_eq!(parsed.fuel, 0.75);
    assert_eq!(parsed.distance_m, 12_345.0);
    assert_eq!(parsed.gear, 4);
}

#[test]
fn rejects_packets_that_are_not_exactly_324_bytes() {
    assert_eq!(
        parse(&[0_u8; PACKET_SIZE - 1]),
        Err(ParseError::WrongSize {
            actual: PACKET_SIZE - 1
        })
    );
}
