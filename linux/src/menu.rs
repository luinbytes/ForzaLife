use crate::locations::LocationKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuPage {
    #[default]
    Closed,
    Main,
    Navigation,
    VehicleCard,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuEffect {
    None,
    ToggleUsage,
    SetNavigation(Option<LocationKind>),
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

    pub fn primary(&mut self) -> MenuEffect {
        match self.page {
            MenuPage::Closed => {
                self.open(MenuPage::Main);
                MenuEffect::None
            }
            MenuPage::VehicleCard => {
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
                4 => {
                    self.close();
                    MenuEffect::None
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
            MenuPage::Main => &[0, 1, 2, 4],
            MenuPage::Navigation => &[0, 1, 2, 3],
            MenuPage::Closed | MenuPage::VehicleCard => return,
        };
        let position = enabled
            .iter()
            .position(|&index| index == self.selected)
            .unwrap_or(0);
        self.selected =
            enabled[(position as i32 + direction).rem_euclid(enabled.len() as i32) as usize];
    }
}
