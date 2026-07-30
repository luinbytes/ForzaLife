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
fn odometer_input_is_available_from_the_main_menu() {
    let mut menu = MenuState::default();
    menu.primary();
    menu.down();
    menu.down();
    menu.down();
    assert_eq!(menu.selected(), 3);
    assert_eq!(menu.primary(), MenuEffect::OpenOdometerInput);
    assert_eq!(menu.page(), MenuPage::OdometerInput);
}

#[test]
fn fuel_test_input_is_available_and_left_returns_to_the_menu() {
    let mut menu = MenuState::default();
    menu.primary();
    menu.down();
    menu.down();
    menu.down();
    menu.down();
    assert_eq!(menu.selected(), 4);
    assert_eq!(menu.primary(), MenuEffect::OpenFuelInput);
    assert_eq!(menu.page(), MenuPage::FuelInput);

    menu.back();
    assert_eq!(menu.page(), MenuPage::Main);
}

#[test]
fn reload_overlay_is_the_last_main_menu_action() {
    let mut menu = MenuState::default();
    menu.primary();
    for _ in 0..8 {
        menu.down();
    }
    assert_eq!(menu.selected(), 8);
    assert_eq!(menu.primary(), MenuEffect::ReloadOverlay);
    assert_eq!(menu.page(), MenuPage::Closed);
}

#[test]
fn main_menu_can_reset_session_and_cycle_hud_mode() {
    let mut menu = MenuState::default();
    menu.primary();
    for _ in 0..5 {
        menu.down();
    }
    assert_eq!(menu.primary(), MenuEffect::ResetSession);

    menu.primary();
    for _ in 0..6 {
        menu.down();
    }
    assert_eq!(menu.primary(), MenuEffect::CycleHudMode);
}
