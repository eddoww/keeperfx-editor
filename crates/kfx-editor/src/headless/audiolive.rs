//! `--actions` of the sound previews and the live link (one dispatch line in the action loop,
//! `headless/actions.rs`).
//!
//! * `{"sounds": {"tab": "sounds" | "creature", "creature": "IMP", "filter": "gold", "category":
//!   "effects" | "speech" | "creatures" | "music", "play": "fx:32"}}`: opens the configuration
//!   editor on its Sounds tab (or a creature's Sounds page), loads the library and sets the
//!   browser; `play` plays an entry (on the null backend in this mode).
//! * `{"mute": true}`: Preferences > Sound > mute.
//! * `{"speech_preview": "intro" | "outro" | "<file>"}`: Level properties' speech preview.
//! * `{"live": {"open": true, "connect": "127.0.0.1:5599", "tab": "events" | "watch" | "console" |
//!   "actions" | "positions", "api_on_play": true, "port": 5599, "overlay": true,
//!   "disconnect": true}}`.
//! * `{"live_demo": true}`: fills the panel with a recorded session (events, watches, positions)
//!   for documentation screenshots.
//! * `{"print_audiolive": true}`: prints the browser / live state to stderr.
use crate::app::EditorApp;
use serde_json::{Map, Value};

pub fn run_audiolive(app: &mut EditorApp, obj: &Map<String, Value>) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    let b = |v: &Value| v.as_bool().unwrap_or(true);
    if let Some(v) = get("mute") {
        app.settings.audiolive.mute = b(v);
    }
    if let Some(v) = get("sounds").and_then(|v| v.as_object()) {
        let tab = v.get("tab").and_then(|x| x.as_str()).unwrap_or("sounds");
        let t = if tab == "creature" {
            kfx_config_ui::ui::Tab::Creatures
        } else {
            kfx_config_ui::ui::Tab::Sounds
        };
        app.open_config_editor(Some(t));
        app.ensure_audio()?;
        let creature = v
            .get("creature")
            .and_then(|x| x.as_str())
            .map(|n| n.to_ascii_uppercase());
        if let (Some(c), Some(n)) = (app.config.as_mut(), &creature) {
            c.editor.creature.name = n.clone();
            c.editor.creature.page = kfx_config_ui::ui::creature::Page::Sounds;
        }
        // bind the browser to its place first (that resets the filter), then apply the filter
        match &creature {
            Some(n) if tab == "creature" => {
                app.bind_sound_place(kfx_config_ui::ui::SoundPlace::Creature { name: n })
            }
            _ => app.bind_sound_place(kfx_config_ui::ui::SoundPlace::SoundsTab),
        }
        let mut br = app.sound_browser();
        let br = br.as_mut().ok_or("the sound library did not load")?;
        if let Some(f) = v.get("filter").and_then(|x| x.as_str()) {
            br.filter = f.to_string();
        }
        if let Some(c) = v.get("category").and_then(|x| x.as_str()) {
            br.category = kfx_audio::Category::ALL
                .iter()
                .copied()
                .find(|k| k.label().eq_ignore_ascii_case(c));
        }
        if let Some(k) = v.get("play").and_then(|x| x.as_str()) {
            br.play(k);
            eprintln!("sounds: {}", br.status);
        }
    }
    if let Some(v) = get("speech_preview").and_then(|v| v.as_str()) {
        let name = match v {
            "intro" | "outro" => {
                let (a, b) = app.props_speech().ok_or("the level has no SPEECH")?;
                if v == "intro" {
                    a
                } else {
                    b
                }
            }
            f => f.to_string(),
        };
        app.preview_level_speech(&name);
        eprintln!("speech: {}", app.status);
    }
    if let Some(v) = get("live").and_then(|v| v.as_object()) {
        let s = &mut app.settings.audiolive;
        if let Some(x) = v.get("api_on_play") {
            s.api_on_play = b(x);
        }
        if let Some(x) = v.get("port").and_then(|x| x.as_u64()) {
            s.port = x.clamp(1, 65535) as u16;
        }
        if let Some(x) = v.get("overlay") {
            s.overlay = b(x);
        }
        if let Some(x) = v.get("open") {
            s.live_open = b(x);
        }
        if v.get("disconnect").is_some_and(b) {
            app.live_disconnect();
        }
        if let Some(a) = v.get("connect").and_then(|x| x.as_str()) {
            app.live_connect(a)?;
        }
        if let Some(t) = v.get("tab").and_then(|x| x.as_str()) {
            use kfx_live::ui::Tab;
            let tab = match t {
                "watch" => Tab::Watch,
                "console" => Tab::Console,
                "actions" => Tab::Actions,
                "positions" => Tab::Things,
                _ => Tab::Events,
            };
            if let Some(c) = app.live.conn.as_mut() {
                c.ui.tab = tab;
            }
        }
    }
    if get("live_demo").is_some_and(b) {
        app.live_demo();
    }
    if get("print_audiolive").is_some() {
        if let Some(br) = app.sound_browser().as_ref() {
            eprintln!(
                "sound browser: {} entries, {} visible, status '{}', backend {}",
                br.entries().len(),
                br.visible().len(),
                br.status,
                br.player.backend.name()
            );
        }
        if let Some(c) = &app.live.conn {
            let st = c.panel.client.status();
            eprintln!(
                "live: {:?} {} log {} things {}",
                st.state,
                c.addr,
                c.panel.log.len(),
                c.panel.things.len()
            );
        }
    }
    Ok(())
}
