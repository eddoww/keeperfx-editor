//! Sound previews through the real UI (`ui_tests` harness): Configuration > Sounds… shows the
//! sound browser beside the `sounds.cfg` mappings; searching and pressing Play sends the expected
//! sample to the (recording) null backend; Level properties' speech preview and a script
//! `PLAY_MESSAGE` play the files the game would; mute keeps everything silent. No audio device
//! is opened.
use crate::app::audio::RecordingBackend;
use crate::ui_tests::Ui;
use eframe::egui;
use egui_kittest::kittest::Queryable;
use kfx_audio::play::{AudioBackend, NullBackend};
use kfx_audio::{Pcm, Source};
use kfx_formats::testutil;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

type Played = Arc<Mutex<Vec<Arc<Pcm>>>>;

fn recorder(u: &mut Ui) -> Played {
    let played: Played = Default::default();
    let p = played.clone();
    u.app().set_audio_factory(Rc::new(move || {
        Ok(Box::new(RecordingBackend {
            null: NullBackend::new(),
            played: p.clone(),
        }) as Box<dyn AudioBackend>)
    }));
    played
}

fn same(a: &Pcm, b: &Pcm) -> bool {
    a.sample_rate == b.sample_rate && a.channels == b.channels && a.samples == b.samples
}

#[test]
fn ui_sound_browser_plays_on_the_null_backend() {
    let Some(inst) = testutil::install_dir() else {
        return;
    };
    let Some(mut u) = Ui::new("audiolive-audio") else {
        return;
    };
    // the shipped campaign level itself (its speech comes from the campaign); nothing is saved
    let map = testutil::resolve_ci(&inst, "campgns/keeporig/map00001.slb").expect("keeporig map 1");
    u.open(&map);
    let played = recorder(&mut u);
    u.menu("Configuration", "Sounds…");
    u.steps(4);
    assert!(
        u.app().sound_browser().is_some(),
        "the library loads when the Sounds tab asks for it"
    );
    assert_eq!(
        u.app()
            .sound_browser()
            .as_ref()
            .unwrap()
            .player
            .backend
            .name(),
        "output opens on the first play",
        "nothing opened yet"
    );
    // search like a user: the browser's search box is the "Search" field on the right
    let search =
        u.h.get_all_by_label("Search")
            .map(|n| n.rect())
            .max_by(|a, b| a.min.x.total_cmp(&b.min.x))
            .expect("the browser's Search label");
    u.click_at(search.right_center() + egui::vec2(60.0, 0.0));
    u.text("payday1");
    u.steps(3);
    let keys: Vec<String> = {
        let b = u.app().sound_browser();
        let b = b.as_ref().unwrap();
        b.visible()
            .iter()
            .map(|&i| b.entries()[i].key.clone())
            .collect()
    };
    assert_eq!(keys.first().map(String::as_str), Some("fx:32"), "{keys:?}");
    u.shot("sounds-tab");
    u.click_first("Play");
    u.steps(2);
    let (status, expected, backend) = {
        let b = u.app().sound_browser();
        let b = b.as_ref().unwrap();
        let a = b.shared_audio();
        let r = a.resolve_id(32).unwrap();
        (
            b.status.clone(),
            a.decode(&r.variants[0].source).unwrap(),
            b.player.backend.name().to_string(),
        )
    };
    assert!(status.starts_with("playing fx:32"), "{status}");
    assert_eq!(
        backend, "null (recording)",
        "the output was opened on the first play, and it is the null one"
    );
    {
        let p = played.lock().unwrap();
        assert_eq!(p.len(), 1, "one play");
        assert!(
            same(&p[0], &expected),
            "the backend got sound.dat sample 32 ({} frames)",
            expected.frames()
        );
    }
    u.shot("played");

    // a script PLAY_MESSAGE plays the speech message the game would (speech bank index 57)
    let arg = kfx_script_ui::SoundArg {
        command: "PLAY_MESSAGE".into(),
        kind: kfx_script_ui::SoundArgKind::Speech,
        value: "57".into(),
        span: Default::default(),
    };
    u.app().play_script_sound(&arg);
    let speech = {
        let b = u.app().sound_browser();
        let a = b.as_ref().unwrap().shared_audio();
        let r = a.resolve_speech("57").unwrap();
        a.decode(&r.variants[0].source).unwrap()
    };
    {
        let p = played.lock().unwrap();
        assert_eq!(p.len(), 2, "{}", u.app().status);
        assert!(
            same(&p[1], &speech),
            "{}: {} frames at {} Hz vs {} at {}",
            u.app().status,
            p[1].frames(),
            p[1].sample_rate,
            speech.frames(),
            speech.sample_rate
        );
    }

    // Level properties: the intro speech of level 1 from the campaign's speech folder
    u.menu("Level", "Level properties…");
    u.steps(2);
    u.shot("properties");
    u.click_first("Preview level speech (intro)");
    u.steps(2);
    let status = u.app().status.clone();
    assert!(status.starts_with("Playing speech good01"), "{status}");
    let file = {
        let b = u.app().sound_browser();
        let a = b.as_ref().unwrap().shared_audio();
        let p = kfx_audio::context::file_in(&a.ctx.campaign.speech, "good01.mp3")
            .expect("good01.mp3 in the campaign speech folder");
        a.decode(&Source::File(p)).unwrap()
    };
    assert!(same(&played.lock().unwrap()[2], &file));
    u.shot("speech");
    u.click("Cancel");
    assert!(!u.app().anything_dirty(), "nothing to save");

    // muted: nothing reaches the backend
    u.app().settings.audiolive.mute = true;
    u.steps(1);
    u.app().play_script_sound(&arg);
    assert_eq!(played.lock().unwrap().len(), 3, "muted");
    assert!(u.app().status.contains("muted"), "{}", u.app().status);
}
