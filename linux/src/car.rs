use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct CarInfo {
    pub ordinal: i32,
    pub make: String,
    pub model_full: String,
    pub year: i32,
    pub country: String,
    pub fuel_capacity_liters: f32,
}

impl CarInfo {
    pub fn display_name(&self) -> String {
        format!("{} {}", self.make, self.model_full.to_uppercase())
    }

    pub fn tank_capacity_liters(&self, cylinders: i32) -> f32 {
        if self.fuel_capacity_liters >= 10.0 {
            self.fuel_capacity_liters
        } else {
            Self::fallback_tank_capacity_liters(cylinders)
        }
    }

    pub fn fallback_tank_capacity_liters(cylinders: i32) -> f32 {
        match cylinders {
            8.. => 80.0,
            6..=7 => 65.0,
            4..=5 => 55.0,
            _ => 35.0,
        }
    }
}

pub struct CarDatabase {
    cars: HashMap<i32, CarInfo>,
}

impl CarDatabase {
    pub fn bundled() -> Self {
        let cars = include_str!("../assets/cardata.csv")
            .lines()
            .filter_map(parse_car)
            .map(|car| (car.ordinal, car))
            .collect();
        Self { cars }
    }

    pub fn get(&self, ordinal: i32) -> Option<&CarInfo> {
        self.cars.get(&ordinal)
    }

    pub fn len(&self) -> usize {
        self.cars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cars.is_empty()
    }
}

fn parse_car(line: &str) -> Option<CarInfo> {
    let fields: Vec<_> = line.split(',').collect();
    if fields.len() < 11 {
        return None;
    }
    Some(CarInfo {
        ordinal: fields[0].trim_start_matches('\u{feff}').parse().ok()?,
        make: fields[1].trim().to_owned(),
        model_full: fields[2].trim().to_owned(),
        year: fields[6].trim().parse().unwrap_or_default(),
        country: fields[7].trim().to_owned(),
        fuel_capacity_liters: fields[10].trim().parse().unwrap_or_default(),
    })
}
