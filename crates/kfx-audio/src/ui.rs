//! egui sound browser: search, categories, play / stop, waveform thumbnails, duration, source
//! layer.
//!
//! Embed it in the config editor's Sounds tab or the creature editor:
//!
//! ```ignore
//! let mut browser = SoundBrowser::new(Audio::open(ctx), Player::new(Box::new(NullBackend::new())));
//! // every frame:
//! if let Some(pick) = browser.show(ui) {
//!     // the user pressed "Use": pick.value is what to write in the config
//! }
//! ```
//!
//! The browser never opens an audio device by itself: pass the [`Player`] (null backend or the real
//! one) you want. Decoding happens lazily for rows that scroll into view, a few per frame.
use crate::audio::{Audio, CatalogEntry, Category, LayerKind, Source};
use crate::pcm::Pcm;
use crate::play::{PlayOptions, Player};
use egui::{Color32, Sense, Stroke};
use std::collections::HashMap;
use std::sync::Arc;

/// What the "Use" button hands back to the host editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub key: String,
    pub category: Category,
    /// A value suitable for the config: an alias (`GOLD_PICKUP`) or id for effects, the message
    /// name for speech, the file name for files, `creature.slot` for creature sets.
    pub value: String,
    /// `id count` for sound.dat ranges, when the entry is a bank range.
    pub id_count: Option<(u16, usize)>,
}

pub struct SoundBrowser {
    audio: Arc<Audio>,
    pub player: Player,
    entries: Vec<CatalogEntry>,
    pub filter: String,
    /// `None` = all categories.
    pub category: Option<Category>,
    /// `None` = every layer.
    pub layer: Option<LayerKind>,
    pub selected: Option<String>,
    pub volume: f32,
    thumbs: HashMap<String, Vec<(f32, f32)>>,
    durations: HashMap<String, f64>,
    failed: HashMap<String, String>,
    pub status: String,
    variant_counter: HashMap<String, usize>,
}

const ROW_H: f32 = 26.0;
const THUMB_W: f32 = 120.0;
const THUMBS_PER_FRAME: usize = 6;

impl SoundBrowser {
    pub fn new(audio: Audio, player: Player) -> SoundBrowser {
        Self::with_shared(Arc::new(audio), player)
    }

    /// [`SoundBrowser::new`] with a library the host shares (hover text, other previews).
    pub fn with_shared(audio: Arc<Audio>, player: Player) -> SoundBrowser {
        let entries = audio.catalog();
        SoundBrowser {
            audio,
            player,
            entries,
            filter: String::new(),
            category: None,
            layer: None,
            selected: None,
            volume: 1.0,
            thumbs: HashMap::new(),
            durations: HashMap::new(),
            failed: HashMap::new(),
            status: String::new(),
            variant_counter: HashMap::new(),
        }
    }

    /// Replace the library (another campaign / level) and rebuild the list.
    pub fn set_audio(&mut self, audio: Audio) {
        self.set_shared_audio(Arc::new(audio));
    }

    /// [`SoundBrowser::set_audio`] with a shared library.
    pub fn set_shared_audio(&mut self, audio: Arc<Audio>) {
        self.entries = audio.catalog();
        self.audio = audio;
        self.thumbs.clear();
        self.durations.clear();
        self.failed.clear();
        self.player.stop();
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    /// The library, to share with the host.
    pub fn shared_audio(&self) -> Arc<Audio> {
        self.audio.clone()
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Indices of the entries that pass the search box, category and layer filter.
    pub fn visible(&self) -> Vec<usize> {
        let q = self.filter.trim().to_ascii_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.category.is_none_or(|c| c == e.category))
            .filter(|(_, e)| self.layer.is_none_or(|l| l == e.resolved.layer.kind))
            .filter(|(_, e)| {
                q.is_empty()
                    || e.title.to_ascii_lowercase().contains(&q)
                    || e.detail.to_ascii_lowercase().contains(&q)
                    || e.resolved.layer.label.to_ascii_lowercase().contains(&q)
                    || e.resolved
                        .variants
                        .iter()
                        .any(|v| v.label.to_ascii_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn entry(&self, key: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Decode (cached) the variant `n` of an entry.
    fn clip(&mut self, key: &str, variant: usize) -> Option<Arc<Pcm>> {
        let e = self.entry(key)?;
        let v = e
            .resolved
            .variants
            .get(variant % e.resolved.variants.len().max(1))?
            .clone();
        if matches!(v.source, Source::Silence) {
            return None;
        }
        let cache_key = format!("{key}#{}", variant % e.resolved.variants.len().max(1));
        let audio = &self.audio;
        match self.player.clip(&cache_key, || audio.decode(&v.source)) {
            Ok(p) => Some(p),
            Err(err) => {
                self.failed.insert(key.to_string(), err.to_string());
                None
            }
        }
    }

    /// Play an entry (round-robin over its variants when it has several).
    pub fn play(&mut self, key: &str) {
        let n = self
            .entry(key)
            .map(|e| e.resolved.variants.len())
            .unwrap_or(0);
        if n == 0 {
            self.status = format!("{key}: nothing to play");
            return;
        }
        let counter = self.variant_counter.entry(key.to_string()).or_insert(0);
        let variant = *counter % n;
        *counter += 1;
        match self.clip(key, variant) {
            Some(p) => match self.player.play_clip(
                p,
                PlayOptions {
                    volume: self.volume,
                    ..Default::default()
                },
            ) {
                Ok(_) => {
                    self.status = format!(
                        "playing {key}{}",
                        if n > 1 {
                            format!(" (variant {} of {n})", variant + 1)
                        } else {
                            String::new()
                        }
                    )
                }
                Err(e) => self.status = format!("playback failed: {e}"),
            },
            None => {
                self.status = self
                    .failed
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| "silent".into())
            }
        }
    }

    pub fn pick_of(&self, key: &str) -> Option<Pick> {
        let e = self.entry(key)?;
        let first = e.resolved.variants.first();
        let bank_range = match first.map(|v| &v.source) {
            Some(Source::Bank { index, .. })
                if e.resolved.layer.kind == LayerKind::Bank || e.key.starts_with("fx:") =>
            {
                Some((*index, e.resolved.declared_count.max(1)))
            }
            _ => None,
        };
        let value = match e.category {
            Category::Effects => match e.key.strip_prefix("fx:") {
                Some(id) => e
                    .detail
                    .split([',', ';'])
                    .next()
                    .map(|a| a.trim())
                    .filter(|a| {
                        !a.is_empty()
                            && a.chars()
                                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                    })
                    .map(str::to_string)
                    .unwrap_or_else(|| id.to_string()),
                None => e.title.clone(),
            },
            Category::Speech => e.title.split(' ').next().unwrap_or("").to_string(),
            _ => first
                .map(|v| v.label.clone())
                .unwrap_or_else(|| e.title.clone()),
        };
        Some(Pick {
            key: e.key.clone(),
            category: e.category,
            value,
            id_count: bank_range,
        })
    }

    fn thumbnail(&mut self, key: &str) -> Option<&Vec<(f32, f32)>> {
        if !self.thumbs.contains_key(key) {
            let pcm = self.clip(key, 0)?;
            self.durations.insert(key.to_string(), pcm.duration_secs());
            self.thumbs
                .insert(key.to_string(), pcm.waveform(THUMB_W as usize / 2));
        }
        self.thumbs.get(key)
    }

    fn draw_thumb(ui: &mut egui::Ui, wave: Option<&Vec<(f32, f32)>>, failed: bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(THUMB_W, ROW_H - 6.0), Sense::hover());
        let bg = ui.visuals().extreme_bg_color;
        ui.painter().rect_filled(rect, 2.0, bg);
        let col = if failed {
            Color32::from_rgb(200, 80, 80)
        } else {
            ui.visuals().widgets.active.fg_stroke.color
        };
        if let Some(w) = wave {
            let mid = rect.center().y;
            let h = rect.height() * 0.5;
            let step = rect.width() / w.len().max(1) as f32;
            for (i, (lo, hi)) in w.iter().enumerate() {
                let x = rect.left() + (i as f32 + 0.5) * step;
                ui.painter().line_segment(
                    [
                        egui::pos2(x, mid - hi.clamp(-1.0, 1.0) * h),
                        egui::pos2(x, mid - lo.clamp(-1.0, 1.0) * h),
                    ],
                    Stroke::new(1.0, col),
                );
            }
        } else {
            ui.painter().line_segment(
                [rect.left_center(), rect.right_center()],
                Stroke::new(1.0, col.gamma_multiply(0.4)),
            );
        }
    }

    /// Draw the browser. Returns the entry the user chose with "Use" (or a double click).
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Pick> {
        self.player.backend.poll();
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .desired_width(180.0)
                    .hint_text("name, alias, file, layer"),
            );
            if ui.small_button("x").clicked() {
                self.filter.clear();
            }
            ui.separator();
            let mut layers: Vec<LayerKind> = vec![];
            for e in &self.entries {
                if !layers.contains(&e.resolved.layer.kind) {
                    layers.push(e.resolved.layer.kind);
                }
            }
            egui::ComboBox::from_id_salt("kfx_audio_layer")
                .selected_text(self.layer.map(layer_name).unwrap_or("all layers"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.layer, None, "All layers");
                    for l in layers {
                        ui.selectable_value(&mut self.layer, Some(l), layer_name(l));
                    }
                });
        });
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.category.is_none(), "All")
                .clicked()
            {
                self.category = None;
            }
            for c in Category::ALL {
                let n = self.entries.iter().filter(|e| e.category == c).count();
                if ui
                    .selectable_label(self.category == Some(c), format!("{} ({n})", c.label()))
                    .clicked()
                {
                    self.category = Some(c);
                }
            }
        });
        ui.separator();
        let visible = self.visible();
        let mut budget = THUMBS_PER_FRAME;
        let mut to_play: Option<String> = None;
        let mut to_stop = false;
        let mut to_select: Option<String> = None;
        let mut use_it: Option<String> = None;
        let height = (ui.available_height() - 64.0).max(80.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, visible.len(), |ui, range| {
                for &i in &visible[range] {
                    let key = self.entries[i].key.clone();
                    let has_wave = self.thumbs.contains_key(&key);
                    if !has_wave
                        && budget > 0
                        && !self.failed.contains_key(&key)
                        && !self.entries[i].resolved.variants.is_empty()
                    {
                        budget -= 1;
                        self.thumbnail(&key);
                    }
                    let e = &self.entries[i];
                    let dur = e.duration.or_else(|| self.durations.get(&key).copied());
                    let playing_this = self.status.starts_with(&format!("playing {key}"))
                        && self.player.is_playing();
                    ui.horizontal(|ui| {
                        ui.set_min_height(ROW_H);
                        let silent = e.resolved.variants.is_empty();
                        if playing_this {
                            if ui
                                .add(egui::Button::new("Stop").min_size(egui::vec2(40.0, 0.0)))
                                .clicked()
                            {
                                to_stop = true;
                            }
                        } else if ui
                            .add_enabled(
                                !silent,
                                egui::Button::new("Play").min_size(egui::vec2(40.0, 0.0)),
                            )
                            .clicked()
                        {
                            to_play = Some(key.clone());
                        }
                        Self::draw_thumb(ui, self.thumbs.get(&key), self.failed.contains_key(&key));
                        let sel = self.selected.as_deref() == Some(&key);
                        let resp = ui.selectable_label(sel, &e.title);
                        if resp.clicked() {
                            to_select = Some(key.clone());
                        }
                        if resp.double_clicked() {
                            use_it = Some(key.clone());
                        }
                        ui.weak(
                            dur.map(|d| format!("{d:.2}s"))
                                .unwrap_or_else(|| "-".into()),
                        );
                        ui.colored_label(
                            layer_color(e.resolved.layer.kind),
                            layer_name(e.resolved.layer.kind),
                        )
                        .on_hover_text(match &e.resolved.layer.file {
                            Some(f) => format!("{}\n{}", e.resolved.layer.label, f.display()),
                            None => e.resolved.layer.label.clone(),
                        });
                        if !e.detail.is_empty() {
                            ui.weak(&e.detail);
                        }
                        if let Some(err) = self.failed.get(&key) {
                            ui.colored_label(Color32::from_rgb(200, 80, 80), err);
                        }
                    });
                }
            });
        if let Some(k) = to_select {
            self.selected = Some(k);
        }
        if let Some(k) = to_play {
            self.selected = Some(k.clone());
            self.play(&k);
        }
        if to_stop {
            self.player.stop();
            self.status.clear();
        }
        if let Some(k) = use_it {
            picked = self.pick_of(&k);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Stop all").clicked() {
                self.player.backend.stop_all();
                self.player.stop();
            }
            ui.label("Volume");
            if ui
                .add(egui::Slider::new(&mut self.volume, 0.0..=1.5).show_value(false))
                .changed()
            {
                self.player.backend.set_master_volume(self.volume);
            }
            let m = self.player.backend.meter();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(90.0, 14.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
            for (c, p) in m.peak.iter().enumerate() {
                let h = rect.height() / 2.0 - 1.0;
                let y = rect.top() + 1.0 + c as f32 * (h + 1.0);
                let col = if *p > 0.95 {
                    Color32::from_rgb(220, 70, 60)
                } else {
                    Color32::from_rgb(90, 190, 110)
                };
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 1.0, y),
                        egui::vec2((rect.width() - 2.0) * p.clamp(0.0, 1.0), h),
                    ),
                    1.0,
                    col,
                );
            }
            ui.weak(format!(
                "{} voice(s), {}",
                self.player.backend.active_voices(),
                self.player.backend.name()
            ));
            if let Some(k) = self.selected.clone() {
                if ui.button("Use").clicked() {
                    picked = self.pick_of(&k);
                }
            }
        });
        if !self.status.is_empty() {
            ui.weak(&self.status);
        }
        if self.player.is_playing() {
            ui.ctx().request_repaint();
        }
        picked
    }
}

pub fn layer_name(k: LayerKind) -> &'static str {
    match k {
        LayerKind::Bank => "bank",
        LayerKind::Install => "install",
        LayerKind::ModAfterBase => "mod:base",
        LayerKind::Campaign => "campaign",
        LayerKind::ModAfterCampaign => "mod:campaign",
        LayerKind::Level => "level",
        LayerKind::ModAfterMap => "mod:map",
        LayerKind::ModRoot => "mod:root",
    }
}

fn layer_color(k: LayerKind) -> Color32 {
    match k {
        LayerKind::Bank => Color32::from_gray(150),
        LayerKind::Install => Color32::from_rgb(130, 160, 200),
        LayerKind::ModAfterBase
        | LayerKind::ModAfterCampaign
        | LayerKind::ModAfterMap
        | LayerKind::ModRoot => Color32::from_rgb(200, 150, 220),
        LayerKind::Campaign => Color32::from_rgb(220, 180, 90),
        LayerKind::Level => Color32::from_rgb(110, 200, 130),
    }
}
