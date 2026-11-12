//! Game sounds in the editor (`kfx-audio`): one sound library per open level (banks, the
//! `sounds.cfg` layers, custom files, creature sets, level speech), the sound browser embedded in
//! the configuration editor (Sounds tab, creature Sounds page), the speech preview of Level
//! properties, and hover / "Play" on the sound arguments of the DK script (`PLAY_MESSAGE`,
//! `SET_MUSIC`).
//!
//! The library is loaded on demand (the first time something asks for a sound) and the audio
//! output device is opened on the first play only ([`LazyBackend`]), never at startup. Tests and
//! the headless screenshot mode use the null backend; Preferences > Sound mutes playback.
use super::*;
use kfx_audio::play::{AudioBackend, Meter, NullBackend, PlayOptions, Player, VoiceId};
use kfx_audio::ui::SoundBrowser;
use kfx_audio::{Audio, Context as AudioContext, Pcm, Resolved, Source};
use kfx_config_ui::ui::{SoundHost, SoundPick, SoundPlace};
use kfx_prefab::i18n::{trd, trf};
use kfx_script_ui::{SoundArg, SoundArgKind};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Makes the output backend on the first play.
pub type BackendFactory = Rc<dyn Fn() -> Result<Box<dyn AudioBackend>, String>>;

/// An [`AudioBackend`] that opens the real one on the first [`AudioBackend::play`] and refuses to
/// play while muted. Until then it reports silence.
pub struct LazyBackend {
    inner: Option<Box<dyn AudioBackend>>,
    factory: BackendFactory,
    muted: Rc<Cell<bool>>,
    master: f32,
    max_voices: usize,
    /// Why the device could not be opened (then the null backend is used).
    pub error: Option<String>,
}

impl LazyBackend {
    pub fn new(factory: BackendFactory, muted: Rc<Cell<bool>>) -> LazyBackend {
        LazyBackend {
            inner: None,
            factory,
            muted,
            master: 1.0,
            max_voices: 16,
            error: None,
        }
    }
}

impl AudioBackend for LazyBackend {
    fn name(&self) -> &str {
        match &self.inner {
            Some(b) => b.name(),
            None if self.muted.get() => "muted",
            None => "output opens on the first play",
        }
    }
    fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> kfx_audio::Result<VoiceId> {
        if self.muted.get() {
            return Err(kfx_audio::Error::Playback(
                trd(
                    "kfx_editor.sound_is_muted_preferences_sound",
                    "sound is muted (Preferences > Sound)",
                )
                .into(),
            ));
        }
        if self.inner.is_none() {
            let mut b = match (self.factory)() {
                Ok(b) => b,
                Err(e) => {
                    self.error = Some(e);
                    Box::new(NullBackend::new())
                }
            };
            b.set_master_volume(self.master);
            b.set_max_voices(self.max_voices);
            self.inner = Some(b);
        }
        self.inner
            .as_mut()
            .map(|b| b.play(pcm, opts))
            .unwrap_or(Ok(VoiceId(0)))
    }
    fn stop(&mut self, id: VoiceId) {
        if let Some(b) = &mut self.inner {
            b.stop(id)
        }
    }
    fn stop_all(&mut self) {
        if let Some(b) = &mut self.inner {
            b.stop_all()
        }
    }
    fn is_playing(&self, id: VoiceId) -> bool {
        self.inner.as_ref().is_some_and(|b| b.is_playing(id))
    }
    fn active_voices(&self) -> usize {
        self.inner.as_ref().map(|b| b.active_voices()).unwrap_or(0)
    }
    fn set_master_volume(&mut self, volume: f32) {
        self.master = volume;
        if let Some(b) = &mut self.inner {
            b.set_master_volume(volume)
        }
    }
    fn master_volume(&self) -> f32 {
        self.master
    }
    fn set_max_voices(&mut self, n: usize) {
        self.max_voices = n;
        if let Some(b) = &mut self.inner {
            b.set_max_voices(n)
        }
    }
    fn meter(&self) -> Meter {
        self.inner.as_ref().map(|b| b.meter()).unwrap_or_default()
    }
    fn poll(&mut self) {
        if let Some(b) = &mut self.inner {
            b.poll()
        }
    }
    fn position_secs(&self, id: VoiceId) -> Option<f64> {
        self.inner.as_ref().and_then(|b| b.position_secs(id))
    }
}

/// A null backend that also keeps every clip it was asked to play (UI tests).
#[cfg(test)]
pub struct RecordingBackend {
    pub null: NullBackend,
    pub played: Arc<std::sync::Mutex<Vec<Arc<Pcm>>>>,
}

#[cfg(test)]
impl AudioBackend for RecordingBackend {
    fn name(&self) -> &str {
        "null (recording)"
    }
    fn play(&mut self, pcm: Arc<Pcm>, opts: PlayOptions) -> kfx_audio::Result<VoiceId> {
        if let Ok(mut p) = self.played.lock() {
            p.push(pcm.clone());
        }
        self.null.play(pcm, opts)
    }
    fn stop(&mut self, id: VoiceId) {
        self.null.stop(id)
    }
    fn stop_all(&mut self) {
        self.null.stop_all()
    }
    fn is_playing(&self, id: VoiceId) -> bool {
        self.null.is_playing(id)
    }
    fn active_voices(&self) -> usize {
        self.null.active_voices()
    }
    fn set_master_volume(&mut self, volume: f32) {
        self.null.set_master_volume(volume)
    }
    fn master_volume(&self) -> f32 {
        self.null.master_volume()
    }
    fn set_max_voices(&mut self, n: usize) {
        self.null.set_max_voices(n)
    }
    fn meter(&self) -> Meter {
        self.null.meter()
    }
    fn poll(&mut self) {
        self.null.poll()
    }
    fn position_secs(&self, id: VoiceId) -> Option<f64> {
        self.null.position_secs(id)
    }
}

/// What the library depends on: install, campaign / map pack `.cfg`, level, language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioKey {
    pub install: PathBuf,
    pub campaign: Option<PathBuf>,
    pub level: Option<u32>,
    pub language: String,
}

/// The sound library and browser of the open level, shared with the configuration editor's
/// [`SoundHost`] (single thread: `Rc<RefCell<..>>`).
pub struct AudioShared {
    pub key: Option<AudioKey>,
    pub browser: Option<SoundBrowser>,
    /// Something asked for a sound while the library was not loaded: load it next frame.
    pub wanted: bool,
    /// Where the browser was last drawn (`sounds` / `creature:<name>`): a new place resets its
    /// filter.
    place: String,
    factory: BackendFactory,
    pub muted: Rc<Cell<bool>>,
}

/// State of the editor's sound support.
pub struct AudioState {
    pub shared: Rc<RefCell<AudioShared>>,
    /// The library for the script editor's hover text (shared across threads).
    lookup_audio: Arc<std::sync::RwLock<Option<Arc<Audio>>>>,
    /// Set by the script hover while the library is not loaded.
    lookup_wanted: Arc<AtomicBool>,
    lookup: kfx_script_ui::SoundLookup,
}

impl AudioState {
    /// `real_device`: open the system output on the first play (else the null backend).
    pub fn new(real_device: bool) -> AudioState {
        let factory: BackendFactory = if real_device {
            Rc::new(real_backend)
        } else {
            Rc::new(|| Ok(Box::new(NullBackend::new()) as Box<dyn AudioBackend>))
        };
        let lookup_audio: Arc<std::sync::RwLock<Option<Arc<Audio>>>> = Default::default();
        let lookup_wanted = Arc::new(AtomicBool::new(false));
        let (la, lw) = (lookup_audio.clone(), lookup_wanted.clone());
        let lookup: kfx_script_ui::SoundLookup = Arc::new(move |arg: &SoundArg| {
            let g = la.read().ok()?;
            match g.as_ref() {
                Some(a) => Some(describe_arg(a, arg)),
                None => {
                    lw.store(true, Ordering::SeqCst);
                    Some(
                        trd(
                            "kfx_editor.loading_the_game_s_sounds",
                            "Loading the game's sounds…",
                        )
                        .to_string(),
                    )
                }
            }
        });
        AudioState {
            shared: Rc::new(RefCell::new(AudioShared {
                key: None,
                browser: None,
                wanted: false,
                place: String::new(),
                factory,
                muted: Rc::new(Cell::new(false)),
            })),
            lookup_audio,
            lookup_wanted,
            lookup,
        }
    }
}

/// The system's default output (cpal), opened on the first play.
fn real_backend() -> Result<Box<dyn AudioBackend>, String> {
    kfx_audio::play::CpalBackend::new()
        .map(|b| Box::new(b) as Box<dyn AudioBackend>)
        .map_err(|e| {
            trf(
                "kfx_editor.no_audio_output_e",
                "no audio output: {e}",
                &[("e", &e)],
            )
        })
}

/// One line about a resolved sound: what plays, from which layer.
pub fn describe(r: &Resolved) -> String {
    let mut parts = vec![];
    match r.variants.first() {
        None => parts.push(trd("kfx_editor.silent_not_found", "silent / not found").to_string()),
        Some(v) => {
            let src = match &v.source {
                Source::Bank { index, .. } => trf(
                    "kfx_editor.bank_sample_index_label",
                    "bank sample {index} ({label})",
                    &[("index", &index), ("label", &v.label)],
                ),
                Source::File(p) => p.display().to_string(),
                Source::ZipEntry { zip, entry } => trf(
                    "kfx_editor.entry_in_f",
                    "{entry} in {f}",
                    &[
                        ("entry", &entry),
                        (
                            "f",
                            &zip.file_name()
                                .map(|f| f.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        ),
                    ],
                ),
                Source::Silence => "silence".to_string(),
            };
            parts.push(src);
        }
    }
    if r.variants.len() > 1 {
        parts.push(trf(
            "kfx_editor.variants_variants_the_game_picks_one_at_random",
            "{variants} variants, the game picks one at random",
            &[("variants", &r.variants.len())],
        ));
    }
    parts.push(trf(
        "kfx_editor.from_label",
        "from {label}",
        &[("label", &r.layer.label)],
    ));
    if let Some(n) = &r.note {
        parts.push(n.clone());
    }
    parts.join("; ")
}

/// Resolve a script sound argument like the game (see `kfx_script_ui::sounds`).
pub fn resolve_arg(a: &Audio, arg: &SoundArg) -> Result<Resolved, String> {
    let n = arg.number();
    let v = arg.value.trim();
    let r = match (arg.kind, n) {
        (SoundArgKind::Speech, Some(_)) => a.resolve_speech(v),
        (SoundArgKind::Speech, None) => match a.find_speech_file(v) {
            Some((var, how)) => Ok(Resolved {
                query: v.to_string(),
                variants: vec![var],
                declared_count: 1,
                layer: kfx_audio::Layer {
                    kind: kfx_audio::LayerKind::Install,
                    label: how.to_string(),
                    file: None,
                },
                note: None,
            }),
            None => Err(kfx_audio::Error::NotFound(v.to_string())),
        },
        (SoundArgKind::Sound, Some(id)) => a.resolve_id(id),
        (SoundArgKind::Sound, None) => a.resolve(v).or_else(|e| {
            a.find_sound_file(v, false)
                .map(|var| Resolved {
                    query: v.to_string(),
                    variants: vec![var],
                    declared_count: 1,
                    layer: kfx_audio::Layer {
                        kind: kfx_audio::LayerKind::Install,
                        label: trd("kfx_editor.custom_file", "custom file").into(),
                        file: None,
                    },
                    note: None,
                })
                .ok_or(e)
        }),
        (SoundArgKind::Music, Some(t)) => a.resolve_music_track(t),
        (SoundArgKind::Music, None) => a.resolve_music_file(v),
    };
    r.map_err(|e| e.to_string())
}

fn describe_arg(a: &Audio, arg: &SoundArg) -> String {
    match resolve_arg(a, arg) {
        Ok(r) => describe(&r),
        Err(e) => trf(
            "kfx_editor.not_found_for_this_level_e",
            "Not found for this level: {e}",
            &[("e", &e)],
        ),
    }
}

/// The configuration editor's view of the shared browser.
struct EditorSoundHost(Rc<RefCell<AudioShared>>);

/// Show the browser for `place`: a new place resets the filter (all sounds on the Sounds tab,
/// the creature's own sounds on its page).
fn bind_place(s: &mut AudioShared, place: SoundPlace<'_>) {
    let tag = match place {
        SoundPlace::SoundsTab => "sounds".to_string(),
        SoundPlace::Creature { name } => format!("creature:{}", name.to_ascii_lowercase()),
    };
    if s.place == tag {
        return;
    }
    if let Some(b) = s.browser.as_mut() {
        match place {
            SoundPlace::SoundsTab => {
                b.category = None;
                b.filter.clear();
            }
            SoundPlace::Creature { name } => {
                b.category = Some(kfx_audio::Category::Creatures);
                b.filter = name.to_ascii_lowercase();
            }
        }
        s.place = tag;
    }
}

impl SoundHost for EditorSoundHost {
    fn browser(&mut self, ui: &mut egui::Ui, place: SoundPlace<'_>) -> Option<SoundPick> {
        let mut s = self.0.borrow_mut();
        if s.browser.is_none() {
            s.wanted = true;
            ui.label(
                RichText::new(trd(
                    "kfx_editor.loading_the_game_s_sounds",
                    "Loading the game's sounds…",
                ))
                .color(TEXT_DIM),
            );
            ui.ctx().request_repaint();
            return None;
        }
        bind_place(&mut s, place);
        let b = s.browser.as_mut()?;
        b.show(ui).map(|p| SoundPick {
            value: p.value,
            id_count: p.id_count,
        })
    }
}

impl EditorApp {
    /// The key of the sound library for the open level.
    fn audio_key(&self) -> Option<AudioKey> {
        let install = self.install()?.to_path_buf();
        let d = self.doc.as_ref();
        Some(AudioKey {
            install,
            campaign: d.and_then(|d| d.pack_cfg.as_ref().map(|(p, _)| p.clone())),
            level: d.map(|d| d.doc.level),
            language: self.settings.language.to_ascii_lowercase(),
        })
    }

    /// Load (or reload for another level) the sound library now.
    pub fn ensure_audio(&mut self) -> Result<(), String> {
        let key = self
            .audio_key()
            .ok_or(trd("kfx_editor.no_keeperfx_folder", "no KeeperFX folder"))?;
        let shared = self.audio.shared.clone();
        let mut s = shared.borrow_mut();
        s.wanted = false;
        if s.key.as_ref() == Some(&key) && s.browser.is_some() {
            return Ok(());
        }
        let mut ctx = AudioContext::new(&key.install, &key.language);
        if self.settings.ignore_mods {
            ctx = ctx.with_mods(Default::default());
        }
        if let Some(c) = &key.campaign {
            ctx = ctx
                .with_campaign(c)
                .map_err(|e| format!("{}: {e}", c.display()))?;
        }
        if let Some(l) = key.level {
            ctx = ctx.with_level(l);
        }
        let audio = Arc::new(Audio::open(ctx));
        match s.browser.as_mut() {
            Some(b) => b.set_shared_audio(audio.clone()),
            None => {
                let backend = LazyBackend::new(s.factory.clone(), s.muted.clone());
                s.browser = Some(SoundBrowser::with_shared(
                    audio.clone(),
                    Player::new(Box::new(backend)),
                ));
            }
        }
        s.key = Some(key);
        s.place.clear();
        if let Ok(mut g) = self.audio.lookup_audio.write() {
            *g = Some(audio);
        }
        Ok(())
    }

    /// Per frame: load the library when asked, follow the open level, mute, script hover.
    pub(super) fn audio_tick(&mut self) {
        self.audio
            .shared
            .borrow()
            .muted
            .set(self.settings.audiolive.mute);
        let playing = self
            .audio
            .shared
            .borrow()
            .browser
            .as_ref()
            .is_some_and(|b| b.player.backend.active_voices() > 0);
        if self.settings.audiolive.mute && playing {
            self.stop_sounds();
        }
        let wanted = self.audio.shared.borrow().wanted
            || self.audio.lookup_wanted.swap(false, Ordering::SeqCst);
        let loaded_key = self.audio.shared.borrow().key.clone();
        let stale = loaded_key.is_some() && loaded_key != self.audio_key();
        if wanted || stale {
            if let Err(e) = self.ensure_audio() {
                self.audio.shared.borrow_mut().wanted = false;
                self.status = trf("kfx_editor.sounds_e", "Sounds: {e}", &[("e", &e)]);
            }
            self.ctx.request_repaint();
        }
        let lookup = self.audio.lookup.clone();
        if let Some(ws) = self.script_ws() {
            ws.set_sound_lookup(Some(lookup));
        }
        if let Some(c) = self.config.as_mut() {
            if c.editor.sound_host.is_none() {
                c.editor.sound_host = Some(Box::new(EditorSoundHost(self.audio.shared.clone())));
            }
        }
    }

    /// Decode and play a resolved sound through the browser's player (replaces the preview).
    fn play_resolved(&mut self, what: &str, r: &Resolved, detail: bool) {
        let shared = self.audio.shared.clone();
        let mut s = shared.borrow_mut();
        let Some(b) = s.browser.as_mut() else { return };
        let audio = b.shared_audio();
        let decoded = r
            .variants
            .first()
            .filter(|v| !matches!(v.source, Source::Silence))
            .map(|v| {
                b.player
                    .clip(&format!("{:?}", v.source), || audio.decode(&v.source))
            });
        self.status = match decoded {
            None => trf(
                "kfx_editor.what_nothing_to_play_r",
                "{what}: nothing to play ({r})",
                &[("what", &what), ("r", &describe(r))],
            ),
            Some(Err(e)) => format!("{what}: {e}"),
            Some(Ok(pcm)) => match b.player.play_clip(
                pcm,
                PlayOptions {
                    volume: b.volume,
                    ..Default::default()
                },
            ) {
                Ok(_) if detail => trf(
                    "kfx_editor.playing_what_r",
                    "Playing {what}: {r}",
                    &[("what", &what), ("r", &describe(r))],
                ),
                Ok(_) => trf(
                    "kfx_editor.playing_what",
                    "Playing {what}",
                    &[("what", &what)],
                ),
                Err(e) => format!("{what}: {e}"),
            },
        };
    }

    /// "Play" of a script sound argument.
    pub fn play_script_sound(&mut self, arg: &SoundArg) {
        if let Err(e) = self.ensure_audio() {
            self.status = trf("kfx_editor.sounds_e", "Sounds: {e}", &[("e", &e)]);
            return;
        }
        let audio = self
            .audio
            .shared
            .borrow()
            .browser
            .as_ref()
            .map(|b| b.shared_audio());
        let Some(audio) = audio else { return };
        match resolve_arg(&audio, arg) {
            Ok(r) => self.play_resolved(&format!("{} {}", arg.command, arg.value), &r, true),
            Err(e) => self.status = format!("{} {}: {e}", arg.command, arg.value),
        }
    }

    /// Level properties: play a level speech file (`SPEECH`) from the campaign's `[speech]`
    /// folder like the land view does (`play_description_speech`, `src/front_landview.c:836`; `.wav`
    /// is added when the name has no extension).
    pub fn preview_level_speech(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.status = trd("kfx_editor.no_speech_file_set", "No speech file set");
            return;
        }
        if let Err(e) = self.ensure_audio() {
            self.status = trf("kfx_editor.sounds_e", "Sounds: {e}", &[("e", &e)]);
            return;
        }
        let audio = self
            .audio
            .shared
            .borrow()
            .browser
            .as_ref()
            .map(|b| b.shared_audio());
        let Some(audio) = audio else { return };
        let file = if name.contains('.') {
            name.to_string()
        } else {
            format!("{name}.wav")
        };
        let layer = kfx_audio::Layer {
            kind: kfx_audio::LayerKind::Campaign,
            label: trd(
                "kfx_editor.the_campaign_s_speech_folder",
                "the campaign's speech folder",
            )
            .into(),
            file: audio.ctx.campaign.speech.clone(),
        };
        match kfx_audio::context::file_in(&audio.ctx.campaign.speech, &file) {
            Some(p) => {
                let r = Resolved {
                    query: file.clone(),
                    variants: vec![kfx_audio::Variant {
                        source: Source::File(p),
                        label: file.clone(),
                    }],
                    declared_count: 1,
                    layer,
                    note: None,
                };
                let folder = audio
                    .ctx
                    .campaign
                    .speech
                    .as_ref()
                    .and_then(|d| d.file_name())
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.play_resolved(
                    &trf(
                        "kfx_editor.speech_file_from_folder",
                        "speech {file} from {folder}",
                        &[("file", &file), ("folder", &folder)],
                    ),
                    &r,
                    false,
                );
            }
            None => {
                self.status = match &audio.ctx.campaign.speech {
                    Some(d) => trf(
                        "kfx_editor.speech_file_is_not_in_d",
                        "Speech '{file}' is not in {d}",
                        &[("file", &file), ("d", &d.display())],
                    ),
                    None => trd(
                        "kfx_editor.the_level_s_campaign_has_no_speech_folder_for_th",
                        "The level's campaign has no [speech] folder for this language",
                    ),
                }
            }
        }
    }

    /// Preferences > Play: sound previews and the live link.
    pub(super) fn prefs_audiolive(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(RichText::new(trd("kfx_editor.sound_previews", "Sound previews")).color(GOLD));
        let s = &mut self.settings.audiolive;
        ui.checkbox(
            &mut s.mute,
            trd(
                "kfx_editor.mute_the_editor_s_sound_previews",
                "Mute the editor's sound previews",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.the_sound_browser_the_speech_preview_and_the_scr",
            "The sound browser, the speech preview and the script editor's Play \
            buttons stay silent; the audio device is never opened.",
        ));
        let dev = self
            .audio
            .shared
            .borrow()
            .browser
            .as_ref()
            .map(|b| b.player.backend.name().to_string())
            .unwrap_or_else(|| {
                trd(
                    "kfx_editor.the_output_opens_on_the_first_play",
                    "the output opens on the first play",
                )
                .into()
            });
        ui.label(
            RichText::new(trf(
                "kfx_editor.output_dev",
                "Output: {dev}",
                &[("dev", &dev)],
            ))
            .small()
            .color(TEXT_DIM),
        );
        ui.separator();
        ui.label(RichText::new(trd("kfx_editor.live_link", "Live link")).color(GOLD));
        ui.checkbox(
            &mut s.api_on_play,
            trd(
                "kfx_editor.play_and_save_and_play_turn_on_the_game_s_api",
                "Play and Save and play turn on the game's API",
            ),
        )
        .on_hover_text(trd(
            "kfx_editor.adds_cfg_api_enabled_on_cfg_api_port_port_to_the",
            "Adds -cfg API_ENABLED=ON -cfg API_PORT=<port> to the game's command \
            line (KeeperFX reads -cfg lines after keeperfx.cfg); your keeperfx.cfg is not \
            changed.",
        ));
        ui.horizontal(|ui| {
            ui.label(trd("kfx_editor.api_port", "API port"));
            ui.add(egui::DragValue::new(&mut s.port).range(1024..=65535));
        });
        ui.checkbox(
            &mut s.auto_connect,
            trd(
                "kfx_editor.connect_the_live_link_panel_when_the_game_starts",
                "Connect the Live link panel when the game starts",
            ),
        );
    }

    /// Bind the browser to a place now (as drawing it there would), so that a filter set next
    /// stays.
    pub fn bind_sound_place(&mut self, place: SoundPlace<'_>) {
        bind_place(&mut self.audio.shared.borrow_mut(), place);
    }

    /// `SPEECH` (intro, outro) as Level properties shows it (opens the dialog).
    pub fn props_speech(&mut self) -> Option<(String, String)> {
        if self.props_dialog.is_none() {
            self.open_properties();
        }
        self.props_dialog.as_ref()?.info.speech.clone()
    }

    /// Stop every preview.
    pub fn stop_sounds(&mut self) {
        if let Some(b) = self.audio.shared.borrow_mut().browser.as_mut() {
            b.player.backend.stop_all();
            b.player.stop();
        }
    }

    /// Replace how the output is opened (UI tests record what plays).
    #[cfg(test)]
    pub fn set_audio_factory(&mut self, f: BackendFactory) {
        let mut s = self.audio.shared.borrow_mut();
        s.factory = f;
        s.browser = None;
        s.key = None;
    }

    /// The sound browser, once loaded (tests, headless).
    pub fn sound_browser(&mut self) -> std::cell::RefMut<'_, Option<SoundBrowser>> {
        std::cell::RefMut::map(self.audio.shared.borrow_mut(), |s| &mut s.browser)
    }
}
