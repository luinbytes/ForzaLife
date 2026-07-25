use forzalife::{
    locations::LocationKind,
    menu::{MenuEffect, MenuPage, MenuState},
};

#[test]
fn main_menu_matches_the_windows_interaction_flow() {
    let mut menu = MenuState::default();
    assert_eq!(menu.primary(), MenuEffect::None);
    assert_eq!(menu.page(), MenuPage::Main);

    menu.down();
    assert_eq!(menu.selected(), 1);
    assert_eq!(menu.primary(), MenuEffect::None);
    assert_eq!(menu.page(), MenuPage::VehicleCard);

    assert_eq!(menu.primary(), MenuEffect::None);
    assert_eq!(menu.page(), MenuPage::Closed);
}

#[test]
fn navigation_submenu_sets_and_clears_the_target() {
    let mut menu = MenuState::default();
    menu.primary();
    assert_eq!(menu.primary(), MenuEffect::None);
    assert_eq!(menu.page(), MenuPage::Navigation);

    assert_eq!(
        menu.primary(),
        MenuEffect::SetNavigation(Some(LocationKind::Gas))
    );
    assert_eq!(menu.page(), MenuPage::Closed);

    menu.primary();
    menu.primary();
    menu.down();
    menu.down();
    assert_eq!(menu.primary(), MenuEffect::SetNavigation(None));
}

#[test]
fn disabled_jobs_row_is_skipped() {
    let mut menu = MenuState::default();
    menu.primary();
    menu.up();
    assert_eq!(menu.selected(), 4);
    menu.up();
    assert_eq!(menu.selected(), 2);
}
