//! The live panel drawn headless (egui `Context::run_ui`, no window / GPU) against the mock game.
#![cfg(feature = "ui")]
mod support;
use kfx_live::client::{Config, LiveClient};
use kfx_live::model::LivePanel;
use kfx_live::proto::Player;
use kfx_live::ui::{self, Tab, UiState};
use std::time::{Duration, Instant};
use support::Mock;

fn frame(ctx: &egui::Context, p: &mut LivePanel, st: &mut UiState) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    ctx.run_ui(input, |ui| ui::show(ui, p, st))
        .drop_without_applying_deltas();
}

#[test]
fn every_tab_draws_with_and_without_a_game() {
    let ctx = egui::Context::default();
    let mut st = UiState::default();
    // without a game
    let mut cfg = Config::localhost(1); // port 1: nothing listens
    cfg.reconnect_interval = Duration::from_millis(200);
    cfg.connect_timeout = Duration::from_millis(100);
    let mut p = LivePanel::new(LiveClient::spawn(cfg));
    for tab in [
        Tab::Events,
        Tab::Watch,
        Tab::Console,
        Tab::Actions,
        Tab::Things,
    ] {
        st.tab = tab;
        frame(&ctx, &mut p, &mut st);
    }
    // with the mock
    let m = Mock::start();
    let mut p = LivePanel::new(LiveClient::spawn(Config::localhost(m.port)));
    p.add_watch(Player::P0, "FLAG0");
    p.add_watch(Player::Index(1), "FLAG0");
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && !p.client.is_connected() {
        std::thread::sleep(Duration::from_millis(20));
    }
    // the panel subscribes when it processes the Connected event: wait for that before emitting
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && !m.with(|s| s.sub_events.iter().any(|e| e == "GAME_SAVED")) {
        frame(&ctx, &mut p, &mut st);
        std::thread::sleep(Duration::from_millis(20));
    }
    m.set_var(0, "FLAG0", 3);
    m.emit("GAME_SAVED", None);
    for _ in 0..30 {
        for tab in [
            Tab::Events,
            Tab::Watch,
            Tab::Console,
            Tab::Actions,
            Tab::Things,
        ] {
            st.tab = tab;
            frame(&ctx, &mut p, &mut st);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(p.client.is_connected());
    assert!(p.log.iter().any(|e| e.tag == "GAME_SAVED"));
    let w = p.watches[0].clone();
    assert_eq!(p.values[&w].value, Some(3));
}

/// The watch player list offers the heroes (a hero wave's creatures can be watched).
#[test]
fn watch_players_include_heroes_and_neutral() {
    let names: Vec<String> = ui::watch_players().iter().map(|p| p.name()).collect();
    assert_eq!(names.first().map(String::as_str), Some("PLAYER0"));
    assert!(names.iter().any(|n| n == "PLAYER6"), "{names:?}");
    assert!(names.iter().any(|n| n == "PLAYER_GOOD"), "{names:?}");
    assert!(names.iter().any(|n| n == "PLAYER_NEUTRAL"), "{names:?}");
}
