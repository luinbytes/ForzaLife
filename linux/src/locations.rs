use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationKind {
    Gas,
    Workshop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub name: String,
    pub kind: LocationKind,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Locations {
    pub locations: Vec<Location>,
}

impl Locations {
    pub fn nearest(&self, kind: LocationKind, position: [f32; 3]) -> Option<(&Location, f32)> {
        self.locations
            .iter()
            .filter(|location| location.kind == kind)
            .map(|location| (location, distance(location.position, position)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
    }

    pub fn add(&mut self, kind: LocationKind, position: [f32; 3]) {
        let number = self
            .locations
            .iter()
            .filter(|location| location.kind == kind)
            .count()
            + 1;
        let label = match kind {
            LocationKind::Gas => "Gas station",
            LocationKind::Workshop => "Workshop",
        };
        self.locations.push(Location {
            name: format!("{label} {number}"),
            kind,
            position,
        });
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

fn distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2) + (left[2] - right[2]).powi(2))
        .sqrt()
}
