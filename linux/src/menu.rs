use crate::locations::LocationKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuPage {
    #[default]
    Closed,
    Main,
    Navigation,
    VehicleCard,
    OdometerInput,
    FuelInput,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuEffect {
    None,
    ReloadOverlay,
    ToggleUsage,
    OpenOdometerInput,
    OpenFuelInput,
    SetNavigation(Option<LocationKind>),
    CycleHudMode,
    ResetSession,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudMode {
    #[default]
    Life,
    Drive,
    Minimal,
}

#[derive(Default)]
pub struct MenuState {
    page: MenuPage,
    selected: usize,
}

impl MenuState {
    pub fn page(&self) -> MenuPage {
        self.page
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn up(&mut self) {
        self.move_selection(-1);
    }

    pub fn down(&mut self) {
        self.move_selection(1);
    }

    pub fn close(&mut self) {
        self.page = MenuPage::Closed;
    }

    pub fn back(&mut self) {
        match self.page {
            MenuPage::Closed => {}
            MenuPage::Main => self.close(),
            MenuPage::Navigation
            | MenuPage::VehicleCard
            | MenuPage::OdometerInput
            | MenuPage::FuelInput => self.open(MenuPage::Main),
        }
    }

    pub fn primary(&mut self) -> MenuEffect {
        match self.page {
            MenuPage::Closed => {
                self.open(MenuPage::Main);
                MenuEffect::None
            }
            MenuPage::VehicleCard | MenuPage::OdometerInput | MenuPage::FuelInput => {
                self.close();
                MenuEffect::None
            }
            MenuPage::Main => match self.selected {
                0 => {
                    self.open(MenuPage::Navigation);
                    MenuEffect::None
                }
                1 => {
                    self.open(MenuPage::VehicleCard);
                    MenuEffect::None
                }
                2 => {
                    self.close();
                    MenuEffect::ToggleUsage
                }
                3 => {
                    self.open(MenuPage::OdometerInput);
                    MenuEffect::OpenOdometerInput
                }
                4 => {
                    self.open(MenuPage::FuelInput);
                    MenuEffect::OpenFuelInput
                }
                5 => {
                    self.close();
                    MenuEffect::ResetSession
                }
                6 => {
                    self.close();
                    MenuEffect::CycleHudMode
                }
                7 => {
                    self.close();
                    MenuEffect::None
                }
                8 => {
                    self.close();
                    MenuEffect::ReloadOverlay
                }
                _ => MenuEffect::None,
            },
            MenuPage::Navigation => match self.selected {
                0 => {
                    self.close();
                    MenuEffect::SetNavigation(Some(LocationKind::Gas))
                }
                1 => {
                    self.close();
                    MenuEffect::SetNavigation(Some(LocationKind::Workshop))
                }
                2 => {
                    self.close();
                    MenuEffect::SetNavigation(None)
                }
                3 => {
                    self.open(MenuPage::Main);
                    MenuEffect::None
                }
                _ => MenuEffect::None,
            },
        }
    }

    fn open(&mut self, page: MenuPage) {
        self.page = page;
        self.selected = 0;
    }

    fn move_selection(&mut self, direction: i32) {
        let enabled: &[usize] = match self.page {
            MenuPage::Main => &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            MenuPage::Navigation => &[0, 1, 2, 3],
            MenuPage::Closed
            | MenuPage::VehicleCard
            | MenuPage::OdometerInput
            | MenuPage::FuelInput => return,
        };
        let position = enabled
            .iter()
            .position(|&index| index == self.selected)
            .unwrap_or(0);
        self.selected =
            enabled[(position as i32 + direction).rem_euclid(enabled.len() as i32) as usize];
    }
}
