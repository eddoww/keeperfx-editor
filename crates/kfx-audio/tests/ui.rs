//! The egui browser, run headless (egui's `Context::run` needs no window or GPU) on the install's
//! sounds.
#![cfg(feature = "ui")]
use kfx_audio::play::{NullBackend, Player};
use kfx_audio::ui::SoundBrowser;
use kfx_audio::{Audio, Category, Context};
use kfx_formats::testutil::{install_dir, resolve_ci};

fn browser() -> Option<SoundBrowser> {
    let install = install_dir()?;
    let cfg = resolve_ci(&install, "campgns/keeporig.cfg")?;
    let ctx = Context::new(&install, "eng")
        .with_campaign(&cfg)
        .ok()?
        .with_level(1);
    Some(SoundBrowser::new(
        Audio::open(ctx),
        Player::new(Box::new(NullBackend::new())),
    ))
}

fn frame(ctx: &egui::Context, b: &mut SoundBrowser) -> Option<kfx_audio::ui::Pick> {
    let mut out = None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    ctx.run_ui(input, |ui| {
        out = b.show(ui);
    })
    .drop_without_applying_deltas();
    out
}

#[test]
fn filters_play_and_pick() {
    let Some(mut b) = browser() else { return };
    let ctx = egui::Context::default();
    for _ in 0..5 {
        frame(&ctx, &mut b);
    }
    let all = b.visible().len();
    assert!(all > 1500, "{all}");
    // search by alias, by file name, by category
    b.filter = "gold_pickup".into();
    let v = b.visible();
    assert_eq!(
        v.len(),
        3,
        "fx:32..34 carry the GOLD_PICKUP alias: {:?}",
        v.iter().map(|&i| &b.entries()[i].title).collect::<Vec<_>>()
    );
    b.filter = "foot1a".into();
    assert!(!b.visible().is_empty());
    b.filter.clear();
    b.category = Some(Category::Creatures);
    assert!(b
        .visible()
        .iter()
        .all(|&i| b.entries()[i].category == Category::Creatures));
    b.category = Some(Category::Music);
    assert!(b.visible().is_empty(), "the install has no music files");
    b.category = Some(Category::Effects);
    b.filter = "imp".into();
    for _ in 0..3 {
        frame(&ctx, &mut b);
    }
    b.filter = "gold_pickup".into();
    // visible rows get thumbnails (a few per frame)
    for _ in 0..4 {
        frame(&ctx, &mut b);
    }
    // play on the null backend
    b.play("fx:32");
    assert!(b.player.is_playing(), "{}", b.status);
    frame(&ctx, &mut b);
    assert!(b.status.starts_with("playing fx:32"));
    b.player.stop();
    // the value a config editor gets: the alias if there is one, else the id; speech gives the
    // message name
    let p = b.pick_of("fx:32").unwrap();
    assert_eq!(
        (p.value.as_str(), p.id_count),
        ("GOLD_PICKUP", Some((32, 1)))
    );
    assert_eq!(b.pick_of("fx:4").unwrap().value, "4");
    let sp = b
        .entries()
        .iter()
        .find(|e| e.category == Category::Speech && e.title.starts_with("GoldLow"))
        .unwrap()
        .key
        .clone();
    assert_eq!(b.pick_of(&sp).unwrap().value, "GoldLow");
    // a speech clip with a decodable sample also plays
    b.play(&sp);
    assert!(b.player.is_playing());
    // multi-variant entries round-robin
    let foot = b
        .entries()
        .iter()
        .find(|e| e.key == "creature:imp:Foot")
        .unwrap();
    assert_eq!(foot.resolved.variants.len(), 4);
    b.play("creature:imp:Foot");
    b.play("creature:imp:Foot");
    assert!(b.status.contains("variant 2 of 4"), "{}", b.status);
}
