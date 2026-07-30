pub const PACKET_SIZE: usize = 324;

#[derive(Clone, Debug, PartialEq)]
pub struct Telemetry {
    pub race_on: bool,
    pub timestamp_ms: u32,
    pub engine_max_rpm: f32,
    pub engine_idle_rpm: f32,
    pub current_engine_rpm: f32,
    pub yaw: f32,
    pub power_w: f32,
    pub torque_nm: f32,
    pub race_position: u8,
    pub throttle: u8,
    pub car_ordinal: i32,
    pub num_cylinders: i32,
    pub position: [f32; 3],
    pub speed_mps: f32,
    pub boost_psi: f32,
    pub fuel: f32,
    pub distance_m: f32,
    pub gear: u8,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    WrongSize { actual: usize },
}

pub fn parse(packet: &[u8]) -> Result<Telemetry, ParseError> {
    if packet.len() != PACKET_SIZE {
        return Err(ParseError::WrongSize {
            actual: packet.len(),
        });
    }

    Ok(Telemetry {
        race_on: i32_at(packet, 0) == 1,
        timestamp_ms: u32_at(packet, 4),
        engine_max_rpm: f32_at(packet, 8),
        engine_idle_rpm: f32_at(packet, 12),
        current_engine_rpm: f32_at(packet, 16),
        yaw: f32_at(packet, 56),
        power_w: f32_at(packet, 260),
        torque_nm: f32_at(packet, 264),
        race_position: packet[314],
        throttle: packet[315],
        car_ordinal: i32_at(packet, 212),
        num_cylinders: i32_at(packet, 228),
        position: [
            f32_at(packet, 244),
            f32_at(packet, 248),
            f32_at(packet, 252),
        ],
        speed_mps: f32_at(packet, 256),
        boost_psi: f32_at(packet, 284),
        fuel: f32_at(packet, 288),
        distance_m: f32_at(packet, 292),
        gear: packet[319],
    })
}

fn i32_at(packet: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(packet[offset..offset + 4].try_into().unwrap())
}

fn u32_at(packet: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(packet[offset..offset + 4].try_into().unwrap())
}

fn f32_at(packet: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(packet[offset..offset + 4].try_into().unwrap())
}
