//! The egui side of [`ScriptEditor`]: layout, the text area and its overlays.
use crate::editor::*;
use crate::lang::*;
use crate::latin1;
use crate::textops::{self, Transaction};
use crate::theme::Theme;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{
    vec2, Align, Color32, FontId, Frame, Galley, Id, Key, Margin, Modifiers, Pos2, Rect,
    ScrollArea, Sense, Shape, Stroke, TextBuffer, TextFormat, Ui,
};
use kfx_script::{ConstantProvider, Severity, Span};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Layout job of the whole text: one section per run of equally coloured characters.
#[doc(hidden)]
pub fn build_job(
    text: &str,
    tokens: &[(Span, HlClass)],
    theme: &Theme,
    font: &FontId,
    row_h: f32,
) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, LayoutSection, TextWrapping};
    let ascii = text.is_ascii();
    let byte_of: Vec<usize> = if ascii {
        vec![]
    } else {
        text.char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(text.len()))
            .collect()
    };
    let b = |ci: usize| -> usize {
        if ascii {
            ci.min(text.len())
        } else {
            byte_of[ci.min(byte_of.len() - 1)]
        }
    };
    let mk = |color: Color32, italics: bool| TextFormat {
        font_id: font.clone(),
        color,
        italics,
        line_height: Some(row_h),
        ..Default::default()
    };
    let mut job = LayoutJob {
        text: text.to_string(),
        wrap: TextWrapping::no_max_width(),
        break_on_newline: true,
        ..Default::default()
    };
    job.keep_trailing_whitespace = true;
    let mut pos = 0usize;
    let mut cur: Option<(usize, usize, Color32, bool)> = None;
    let flush = |job: &mut LayoutJob, cur: &mut Option<(usize, usize, Color32, bool)>| {
        if let Some((s, e, c, it)) = cur.take() {
            if e > s {
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: egui::text::ByteIndex(s)..egui::text::ByteIndex(e),
                    format: mk(c, it),
                });
            }
        }
    };
    let push = |job: &mut LayoutJob,
                cur: &mut Option<(usize, usize, Color32, bool)>,
                s: usize,
                e: usize,
                c: Color32,
                it: bool| {
        if e <= s {
            return;
        }
        match cur {
            Some((cs, ce, cc, ci)) if *ce == s && *cc == c && *ci == it => {
                let _ = cs;
                *ce = e;
            }
            _ => {
                flush(job, cur);
                *cur = Some((s, e, c, it));
            }
        }
    };
    let n_chars = if ascii { text.len() } else { byte_of.len() - 1 };
    for (sp, class) in tokens {
        let (s, e) = (sp.start.min(n_chars), sp.end.min(n_chars));
        if s < pos {
            continue;
        }
        push(&mut job, &mut cur, b(pos), b(s), theme.text, false);
        push(
            &mut job,
            &mut cur,
            b(s),
            b(e),
            theme.color_for(*class),
            class.italic(),
        );
        pos = e;
    }
    push(&mut job, &mut cur, b(pos), text.len(), theme.text, false);
    flush(&mut job, &mut cur);
    job
}

pub(crate) fn caret_rect(g: &Galley, origin: Pos2, off: usize) -> Rect {
    g.pos_from_cursor(CCursor::new(off))
        .translate(origin.to_vec2())
}

/// Rectangle of a span on its first row (empty spans get a one character box).
pub(crate) fn span_rect(g: &Galley, origin: Pos2, span: Span, char_w: f32) -> Rect {
    let a = g.pos_from_cursor(CCursor::new(span.start));
    let b = g.pos_from_cursor(CCursor::new(span.end));
    let right = if (a.min.y - b.min.y).abs() < 1.0 && span.end > span.start {
        b.min.x
    } else if span.end > span.start {
        let row = g.layout_from_cursor(CCursor::new(span.start)).row;
        g.rows[row].rect().right()
    } else {
        a.min.x + char_w
    };
    Rect::from_min_max(a.min, egui::pos2(right.max(a.min.x + 2.0), a.max.y))
        .translate(origin.to_vec2())
}

pub(crate) fn squiggle(painter: &egui::Painter, r: Rect, color: Color32) {
    let y = r.bottom() - 1.5;
    let mut pts = vec![];
    let mut x = r.left();
    let mut up = false;
    while x < r.right() {
        pts.push(egui::pos2(x, if up { y - 2.0 } else { y }));
        x += 2.0;
        up = !up;
    }
    pts.push(egui::pos2(r.right(), if up { y - 2.0 } else { y }));
    if pts.len() >= 2 {
        painter.add(Shape::line(pts, Stroke::new(1.3, color)));
    }
}

enum Act {
    Down,
    Up,
    Accept,
    CloseCompletion,
    Complete,
    Find,
    Replace,
    Goto,
    Comment,
    Definition,
    Format,
    Escape,
    FindNext,
    FindPrev,
    Enter,
    Tab,
    BackTab,
    /// Ctrl+Home / Ctrl+End (Cmd on macOS): caret to the document start / end; `extend` keeps
    /// the anchor (Shift).
    DocEdge {
        end: bool,
        extend: bool,
    },
}

impl ScriptEditor {
    pub(crate) fn text_id(&self) -> Id {
        self.id.with("text")
    }

    fn ours_focused(&self, ui: &Ui) -> (bool, bool) {
        let t = ui.memory(|m| m.has_focus(self.text_id()));
        let o = t
            || ui.memory(|m| {
                m.has_focus(self.id.with("find"))
                    || m.has_focus(self.id.with("replace"))
                    || m.has_focus(self.id.with("goto"))
            });
        (t, o)
    }

    /// Show the editor in `ui` (fills the available space). Returns the events of this frame.
    pub fn show(&mut self, ui: &mut Ui, host: &dyn ConstantProvider) -> Vec<ScriptEvent> {
        let now = ui.input(|i| i.time);
        self.apply_pending();
        self.update(now, host);
        let theme = self.theme.clone();
        ui.scope(|ui| {
            let v = ui.visuals_mut();
            if v.dark_mode != theme.is_dark {
                *v = if theme.is_dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                };
            }
            v.panel_fill = theme.panel;
            v.extreme_bg_color = theme.background;
            v.selection.bg_fill = theme.selection;
            v.selection.stroke = Stroke::new(1.0, theme.accent);
            v.hyperlink_color = theme.accent;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.accent);
            v.widgets.active.bg_stroke = Stroke::new(1.0, theme.accent);
            v.widgets.inactive.weak_bg_fill = theme.background.gamma_multiply(1.15);
            self.layout(ui, host, now);
        });
        self.was_ours_focused = self.ours_focused(ui).1;
        if self.validation_pending() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    self.options.validate_debounce.max(0.05),
                ));
        }
        let g = self.gen();
        if g != self.reported_gen {
            self.reported_gen = g;
            self.events.push(ScriptEvent::Changed);
        }
        std::mem::take(&mut self.events)
    }

    fn layout(&mut self, ui: &mut Ui, host: &dyn ConstantProvider, now: f64) {
        let theme = self.theme.clone();
        let pframe = Frame::new()
            .fill(theme.panel)
            .inner_margin(Margin::symmetric(8, 4));
        egui::Panel::top(self.id.with("top"))
            .frame(pframe)
            .show_separator_line(true)
            .show(ui, |ui| {
                self.toolbar(ui);
                if self.find.open {
                    self.find_bar(ui);
                }
                if self.goto.open {
                    self.goto_bar(ui);
                }
            });
        egui::Panel::bottom(self.id.with("status"))
            .frame(pframe)
            .show_separator_line(true)
            .show(ui, |ui| {
                self.status_bar(ui);
            });
        if self.problems_open {
            egui::Panel::bottom(self.id.with("problems"))
                .frame(
                    Frame::new()
                        .fill(theme.panel)
                        .inner_margin(Margin::symmetric(8, 4)),
                )
                .resizable(true)
                .default_size(120.0)
                .min_size(50.0)
                .show(ui, |ui| self.problems_panel(ui));
        }
        match self.side {
            SidePanel::None => {}
            s => {
                egui::Panel::right(self.id.with("side"))
                    .frame(
                        Frame::new()
                            .fill(theme.panel)
                            .inner_margin(Margin::symmetric(8, 6)),
                    )
                    .resizable(true)
                    .default_size(330.0)
                    .min_size(180.0)
                    .show(ui, |ui| match s {
                        SidePanel::Outline => self.outline_panel(ui),
                        SidePanel::Snippets => self.snippets_panel(ui),
                        _ => self.commands_panel(ui),
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(Frame::NONE)
            .show(ui, |ui| self.editor_area(ui, host, now));
    }

    fn handle_keys(&mut self, ui: &Ui, host: &dyn ConstantProvider) {
        // egui drops keyboard focus on Escape before we run; popups and bars still get closed
        // by it, and the editor keeps its focus.
        if self.was_ours_focused && ui.input(|i| i.key_pressed(Key::Escape)) {
            if self.completion.is_some() {
                self.completion = None;
                self.want_focus = true;
                return;
            } else if self.find.open || self.goto.open {
                self.find.open = false;
                self.goto.open = false;
                self.want_focus = true;
                return;
            }
        }
        let (text_focused, ours) = self.ours_focused(ui);
        if !ours {
            return;
        }
        if text_focused && self.options.auto_close && !self.is_read_only() {
            self.intercept_typing(ui);
        }
        let mut acts: Vec<Act> = vec![];
        let popup = self.completion.is_some();
        let enter_accepts = self.lang.enter_accepts_completion()
            || self.completion.as_ref().map_or(false, |c| c.navigated);
        ui.input_mut(|i| {
            let none = Modifiers::NONE;
            if text_focused && popup {
                if i.consume_key(none, Key::ArrowDown) {
                    acts.push(Act::Down);
                }
                if i.consume_key(none, Key::ArrowUp) {
                    acts.push(Act::Up);
                }
                if i.consume_key(none, Key::Tab) {
                    acts.push(Act::Accept);
                }
                // Enter accepts when the language wants it or the user picked an item with the
                // arrows
                if enter_accepts && i.consume_key(none, Key::Enter) {
                    acts.push(Act::Accept);
                }
                if i.consume_key(none, Key::Escape) {
                    acts.push(Act::CloseCompletion);
                }
            }
            if i.consume_key(Modifiers::COMMAND, Key::Space) {
                acts.push(Act::Complete);
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) {
                acts.push(Act::Find);
            }
            if i.consume_key(Modifiers::COMMAND, Key::H) {
                acts.push(Act::Replace);
            }
            if i.consume_key(Modifiers::COMMAND, Key::G) {
                acts.push(Act::Goto);
            }
            if i.consume_key(Modifiers::COMMAND, Key::Slash) {
                acts.push(Act::Comment);
            }
            if i.consume_key(Modifiers::ALT | Modifiers::SHIFT, Key::F) {
                acts.push(Act::Format);
            }
            if i.consume_key(none, Key::F12) {
                acts.push(Act::Definition);
            }
            if i.consume_key(none, Key::F3) {
                acts.push(Act::FindNext);
            }
            if i.consume_key(Modifiers::SHIFT, Key::F3) {
                acts.push(Act::FindPrev);
            }
            if i.consume_key(none, Key::Escape) {
                acts.push(Act::Escape);
            }
            if text_focused {
                // egui's text field only knows Ctrl+Home/End with the `ctrl` flag (and ignores
                // macOS Cmd), and the host may see the key before us: handle it here, for any
                // command-like modifier, with and without Shift.
                for (k, end) in [(Key::Home, false), (Key::End, true)] {
                    if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, k) {
                        acts.push(Act::DocEdge { end, extend: true });
                    } else if i.consume_key(Modifiers::COMMAND, k) {
                        acts.push(Act::DocEdge { end, extend: false });
                    }
                }
                if i.consume_key(none, Key::Enter) {
                    acts.push(Act::Enter);
                }
                if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                    acts.push(Act::BackTab);
                }
                if i.consume_key(none, Key::Tab) {
                    acts.push(Act::Tab);
                }
            }
        });
        for a in acts {
            match a {
                Act::Down => {
                    if let Some(c) = &mut self.completion {
                        c.selected = (c.selected + 1) % c.items.len();
                        c.scroll_to_selected = true;
                        c.navigated = true;
                    }
                }
                Act::Up => {
                    if let Some(c) = &mut self.completion {
                        c.selected = (c.selected + c.items.len() - 1) % c.items.len();
                        c.scroll_to_selected = true;
                        c.navigated = true;
                    }
                }
                Act::Accept => {
                    let i = self.completion.as_ref().map(|c| c.selected).unwrap_or(0);
                    self.accept_completion(i);
                }
                Act::CloseCompletion => self.completion = None,
                Act::Complete => {
                    self.completion_armed = true;
                    self.want_focus = true;
                    self.refresh_completions(host);
                }
                Act::Find => self.open_find(false),
                Act::Replace => self.open_find(true),
                Act::Goto => {
                    self.goto.open = true;
                    self.goto.focus = true;
                }
                Act::Comment => self.toggle_comment(),
                Act::Format => self.format_document(),
                Act::Definition => {
                    self.goto_definition();
                }
                Act::Escape => {
                    if self.find.open {
                        self.find.open = false;
                        self.want_focus = true;
                    } else if self.goto.open {
                        self.goto.open = false;
                        self.want_focus = true;
                    }
                }
                Act::FindNext => {
                    self.find_step(true);
                }
                Act::FindPrev => {
                    self.find_step(false);
                }
                Act::Enter => {
                    let tx = self.newline_tx();
                    self.queue(tx);
                }
                Act::Tab => {
                    if !self.snippet_step(true) && !self.tab_nav(true) {
                        self.ensure_cache();
                        let tx = textops::indent_lines(
                            &self.cache.as_ref().unwrap().bytes,
                            self.caret,
                            &self.options.indent.clone(),
                            false,
                        );
                        // a plain Tab with a caret inserts one unit at the caret instead of at the
                        // line start
                        let (lo, hi) = (
                            self.caret.0.min(self.caret.1),
                            self.caret.0.max(self.caret.1),
                        );
                        let multi = self.cache.as_ref().unwrap().bytes[lo..hi].contains(&b'\n');
                        if multi {
                            self.queue(tx);
                        } else {
                            self.insert_at_caret(&self.options.indent.clone());
                        }
                    }
                }
                Act::DocEdge { end, extend } => {
                    let target = if end {
                        self.buf.text().chars().count()
                    } else {
                        0
                    };
                    let anchor = if extend { self.caret.0 } else { target };
                    self.caret = (anchor, target);
                    self.want_sel = Some(self.caret);
                    self.want_scroll = true;
                    self.completion = None;
                }
                Act::BackTab => {
                    if !self.snippet_step(false) && !self.tab_nav(false) {
                        self.ensure_cache();
                        let tx = textops::indent_lines(
                            &self.cache.as_ref().unwrap().bytes,
                            self.caret,
                            &self.options.indent.clone(),
                            true,
                        );
                        self.queue(tx);
                    }
                }
            }
        }
    }

    /// Bracket and quote auto-closing: the typed character (or Backspace) is turned into an edit
    /// and removed from the input so the text field does not see it.
    fn intercept_typing(&mut self, ui: &Ui) {
        let pairs = self.lang.auto_pairs();
        if pairs.is_empty() {
            return;
        }
        self.ensure_cache();
        let bytes = self.cache.as_ref().unwrap().bytes.clone();
        let caret = self.caret;
        let mut tx: Option<(Transaction, Option<char>)> = None;
        ui.input_mut(|i| {
            let texts: Vec<String> = i
                .events
                .iter()
                .filter_map(|e| {
                    if let egui::Event::Text(t) = e {
                        Some(t.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if texts.len() == 1 && texts[0].chars().count() == 1 {
                let ch = texts[0].chars().next().unwrap();
                if let Some(t) = textops::type_char(&bytes, caret, ch, pairs) {
                    i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
                    tx = Some((t, Some(ch)));
                }
            } else if texts.is_empty() {
                if let Some(t) = textops::backspace_pair(&bytes, caret, pairs) {
                    if i.consume_key(Modifiers::NONE, Key::Backspace) {
                        tx = Some((t, None));
                    }
                }
            }
        });
        if let Some((t, ch)) = tx {
            self.queue(t);
            self.typed_override = ch;
        }
    }

    /// After a word was typed: a lone `end` / `else` / `until` moves to its block's indentation.
    fn dedent_typed_word(&mut self) {
        self.ensure_cache();
        let c = self.cache.as_ref().unwrap();
        let caret = self.caret.1.min(c.bytes.len());
        let l = c.lines.line_of(caret);
        let (st, en) = (c.lines.line_start(l), c.lines.line_end(l));
        if caret != en || l == 0 {
            return;
        }
        let word = latin1::decode(&c.bytes[st..en]).trim().to_string();
        if !self.lang.dedent_word(&word) {
            return;
        }
        let mut pl = l;
        let prev_delta = loop {
            if pl == 0 {
                return;
            }
            pl -= 1;
            let t = latin1::decode(&c.bytes[c.lines.line_start(pl)..c.lines.line_end(pl)]);
            if !t.trim().is_empty() {
                break c.analysis.block_delta(&t);
            }
        };
        if let Some(tx) =
            textops::dedent_closing_line(&c.bytes, caret, &self.options.indent, prev_delta)
        {
            self.queue(tx);
            self.apply_pending();
        }
    }

    /// Tab / Shift+Tab between the arguments of the command around the caret.
    fn tab_nav(&mut self, forward: bool) -> bool {
        self.ensure_cache();
        let (lo, hi) = (
            self.caret.0.min(self.caret.1),
            self.caret.0.max(self.caret.1),
        );
        let Some(nav) = self.cache.as_ref().unwrap().analysis.arg_nav(lo) else {
            return false;
        };
        let cur = nav.args.iter().position(|a| a.start <= lo && hi <= a.end);
        let target = if forward {
            match cur {
                Some(i) => nav.args.get(i + 1).copied(),
                None => nav.args.iter().find(|a| a.start >= hi).copied(),
            }
        } else {
            match cur {
                Some(0) => return true,
                Some(i) => nav.args.get(i - 1).copied(),
                None => nav.args.iter().rev().find(|a| a.end <= lo).copied(),
            }
        };
        let sel = match target {
            Some(a) => (a.start, a.end),
            None if forward => (nav.after, nav.after),
            None => return true,
        };
        self.caret = sel;
        self.want_sel = Some(sel);
        self.want_scroll = true;
        true
    }

    fn editor_area(&mut self, ui: &mut Ui, host: &dyn ConstantProvider, now: f64) {
        let theme = self.theme.clone();
        let text_id = self.text_id();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, theme.background);

        self.handle_keys(ui, host);
        // Characters typed this frame decide whether the completion popup opens.
        let (text_focused, _) = self.ours_focused(ui);
        let typed: Option<char> = if text_focused {
            ui.input(|i| {
                i.events.iter().rev().find_map(|e| match e {
                    egui::Event::Text(t) => t.chars().last(),
                    _ => None,
                })
            })
        } else {
            None
        };
        let typed = typed.or(self.typed_override.take());
        let forced_change = self.pending.iter().any(|_| true) && typed.is_some();
        self.apply_pending();
        self.ensure_cache();

        // Push selection / focus requests into the TextEdit state.
        if self.clear_undo || self.want_sel.is_some() {
            let mut st = TextEditState::load(ui.ctx(), text_id).unwrap_or_default();
            if self.clear_undo {
                st.clear_undoer();
                self.clear_undo = false;
            }
            if let Some((a, b)) = self.want_sel.take() {
                st.cursor
                    .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
            }
            st.store(ui.ctx(), text_id);
        }
        if self.want_focus && !self.find.focus && !self.goto.focus {
            ui.memory_mut(|m| m.request_focus(text_id));
            self.want_focus = false;
        }

        let font = FontId::monospace(theme.font_size);
        let (row_h, char_w) = ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, '0')));
        let line_count = self
            .cache
            .as_ref()
            .map(|c| c.lines.line_count())
            .unwrap_or(1);
        let digits = line_count.to_string().len().max(3);
        let gutter_w = if self.options.show_line_numbers {
            digits as f32 * char_w + 30.0
        } else {
            14.0
        };
        let avail = ui.available_size();
        let desired_w = avail.x.max(self.text_width + 40.0);
        let gen_before = self.gen();

        let mut events_out: Vec<ScriptEvent> = vec![];
        let ppp = ui.ctx().pixels_per_point();
        let theme_rev = self.theme_rev;
        let generation = self.generation.clone();
        let lang = &*self.lang;
        let cache = &mut self.cache;
        let galley_cache = &mut self.galley_cache;
        let buf = &mut self.buf;
        let theme_ref = &self.theme;
        let font_size = theme.font_size;
        let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _w: f32| -> Arc<Galley> {
            let gen = generation.load(Ordering::Relaxed);
            if let Some(g) = galley_cache.as_ref() {
                if g.gen == gen
                    && g.ppp == ppp
                    && g.theme_rev == theme_rev
                    && g.font_size == font_size
                {
                    return g.galley.clone();
                }
            }
            if cache.as_ref().map(|c| c.gen) != Some(gen) {
                let bytes = latin1::encode(text.as_str());
                let analysis = lang.analyze(&bytes);
                let lines = textops::LineIndex::new(&bytes);
                *cache = Some(Cache::new(gen, bytes, analysis, lines));
            }
            let c = cache.as_ref().unwrap();
            let job = build_job(
                text.as_str(),
                c.analysis.tokens(),
                theme_ref,
                &FontId::monospace(font_size),
                row_h,
            );
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            *galley_cache = Some(GalleyCache {
                gen,
                ppp,
                font_size,
                theme_rev,
                galley: galley.clone(),
            });
            galley
        };

        let margin = Margin {
            left: (gutter_w + 8.0) as i8,
            right: 24,
            top: 4,
            bottom: 40,
        };
        let mut geometry: Option<TextGeometry> = None;
        let mut new_caret: Option<(usize, usize)> = None;
        let mut text_response: Option<egui::Response> = None;
        let reveal: Option<(usize, usize)> = if self.want_scroll {
            Some(self.caret)
        } else {
            None
        };
        // the editor draws its own gutter; the grey edge fade of the scroll area only smudges it
        ui.style_mut().spacing.scroll.fade.strength = 0.0;
        // the caret or the text changed in the last frames (the editor moves the caret itself
        // for some keys, a frame before the TextEdit lays it out)
        let settling = self.caret_settle > 0;
        let out = ScrollArea::both()
            .id_salt(self.id.with("scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let output = egui::TextEdit::multiline(buf)
                    .id(text_id)
                    .font(font.clone())
                    .text_color(theme.text)
                    .frame(Frame::NONE.inner_margin(margin))
                    .desired_width(desired_w)
                    .min_size(vec2(desired_w, avail.y))
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                if let Some(r) = output.state.cursor.char_range() {
                    new_caret = Some((r.secondary.index.0, r.primary.index.0));
                    // egui keeps the caret inside the clip rectangle, but the gutter is drawn over
                    // the left edge of the text: a caret that moved into the first columns while
                    // the text is scrolled sideways would stay hidden under the line numbers.
                    // Scroll so that it sits right of the gutter (this request comes after the
                    // TextEdit's own).
                    let cr = output
                        .galley
                        .pos_from_cursor(r.primary)
                        .translate(output.galley_pos.to_vec2());
                    let clip = ui.clip_rect();
                    let edge = clip.left() + gutter_w + 8.0;
                    if settling && cr.left() < edge {
                        // (the visible rows as the vertical extent: no vertical scroll)
                        let target = Rect::from_min_max(
                            egui::pos2(cr.left() - gutter_w - 8.0, clip.top()),
                            egui::pos2(cr.left() + 1.0, clip.bottom()),
                        );
                        ui.scroll_to_rect(target, Some(Align::Min));
                    }
                }
                text_response = Some(output.response.response.clone());
                geometry = Some(TextGeometry {
                    galley: output.galley.clone(),
                    pos: output.galley_pos,
                });
                let _ = &mut events_out;
                if let Some((a, b)) = reveal {
                    let r = span_rect(
                        &output.galley,
                        output.galley_pos,
                        Span::new(a.min(b), a.max(b)),
                        char_w,
                    )
                    .expand2(vec2(40.0, 3.0 * row_h));
                    ui.scroll_to_rect(r, Some(Align::Center));
                }
                (output.galley.clone(), output.galley_pos, ui.clip_rect())
            });
        let (galley, gpos, clip) = out.inner;
        let _ = out.content_size;
        self.text_width = galley.size().x + gutter_w + 8.0;
        self.geometry = geometry;
        // egui collapses the selection of an unfocused TextEdit, so only a focused one is the truth
        let focused_now = ui.memory(|m| m.has_focus(text_id));
        if focused_now && self.want_sel.is_none() {
            if let Some(c) = new_caret {
                self.caret = c;
            }
        }
        // fresh analysis for the text as edited this frame
        self.ensure_cache();
        self.update(now, host);
        let changed = self.gen() != gen_before || forced_change;
        if changed
            && typed.map_or(false, |ch| {
                (ch as u32) < 256 && self.lang.is_word_byte(ch as u8)
            })
        {
            self.dedent_typed_word();
        }

        // ---- completion popup state machine
        if changed {
            let mut open = self.completion_armed;
            match typed {
                Some(ch) if (ch as u32) < 256 => {
                    let b = ch as u8;
                    if self.lang.trigger_chars().contains(&ch) {
                        open = true;
                    } else if self.lang.is_word_byte(b) {
                        // inside a snippet placeholder Tab moves on; no popup gets in the way
                        open = self.snippet.is_none() || self.completion.is_some();
                    } else if ch == ' '
                        && self.completion.is_some()
                        && self.lang.space_keeps_completion()
                    {
                        open = true;
                    } else {
                        self.completion = None;
                    }
                }
                Some(_) => self.completion = None,
                None => {
                    if self.completion.is_some() {
                        open = true; // deleting characters keeps the popup in sync
                    }
                }
            }
            if open {
                self.refresh_completions(host);
            }
        } else if self.completion_armed {
            self.refresh_completions(host);
        } else if self.caret != self.prev_caret && self.completion.is_some() {
            self.completion = None;
        }
        self.completion_armed = false;
        self.caret_settle = if self.caret != self.prev_caret || changed {
            3
        } else {
            self.caret_settle.saturating_sub(1)
        };
        self.prev_caret = self.caret;

        // ---- overlays (drawn over the text, inside the scroll area's clip)
        let painter = ui.painter().with_clip_rect(clip);
        let ca = self.caret;
        let caret_line = self
            .cache
            .as_ref()
            .map(|c| c.lines.line_of(ca.1))
            .unwrap_or(0);
        let focused = ui.memory(|m| m.has_focus(text_id));
        // current line
        if let Some(prow) = galley.rows.get(caret_line) {
            let r = prow.rect().translate(gpos.to_vec2());
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(clip.left() + gutter_w, r.top()),
                    egui::pos2(clip.right(), r.bottom()),
                ),
                0.0,
                theme.current_line,
            );
        }
        // find matches
        if self.find.open && !self.find.query.is_empty() {
            let cur = self.find.current;
            for m in self.find_matches() {
                let r = span_rect(&galley, gpos, m, char_w);
                if r.bottom() < clip.top() || r.top() > clip.bottom() {
                    continue;
                }
                painter.rect_filled(
                    r,
                    2.0,
                    if Some(m) == cur {
                        theme.find_current
                    } else {
                        theme.find_match
                    },
                );
            }
        }
        // matching IF / ENDIF
        if let Some((a, b)) = self
            .cache
            .as_ref()
            .and_then(|c| c.analysis.block_match(ca.1))
        {
            for s in [a, b] {
                let r = span_rect(&galley, gpos, s, char_w);
                painter.rect_stroke(
                    r.expand(1.0),
                    2.0,
                    Stroke::new(1.0, theme.accent),
                    egui::StrokeKind::Outside,
                );
                painter.rect_filled(r.expand(1.0), 2.0, theme.block_match);
            }
        }
        // diagnostics: squiggles
        for d in self
            .validation
            .diagnostics
            .iter()
            .filter(|d| self.is_shown(d))
        {
            let r = span_rect(&galley, gpos, d.span, char_w);
            if r.bottom() < clip.top() || r.top() > clip.bottom() {
                continue;
            }
            squiggle(&painter, r, theme.severity_color(d.severity));
        }
        // gutter
        self.paint_gutter(
            ui, &painter, &galley, gpos, clip, gutter_w, caret_line, char_w, &font,
        );

        // ---- reveal (the scroll itself was requested inside the scroll area)
        if reveal.is_some() {
            self.want_scroll = false;
            ui.ctx().request_repaint();
        }

        // ---- ctrl+click on a map reference
        if let Some(resp) = &text_response {
            if resp.clicked() && ui.input(|i| i.modifiers.command) {
                if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                    let off = galley.cursor_from_pos(p - gpos).index.0;
                    let refs = self
                        .cache
                        .as_ref()
                        .map(|c| c.map_refs().to_vec())
                        .unwrap_or_default();
                    if let Some(r) = refs.iter().find(|r| r.span.touches(off)) {
                        self.events.push(ScriptEvent::JumpToMapLocation(r.target));
                    } else {
                        self.follow_definition(off);
                    }
                }
            }
            self.hover_ui(ui, resp, &galley, gpos, clip, char_w, now);
        }

        self.check_snippet_caret();

        // ---- popups
        if focused || self.completion.is_some() {
            self.popups(ui, &galley, gpos, clip, row_h);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_gutter(
        &mut self,
        ui: &mut Ui,
        painter: &egui::Painter,
        galley: &Galley,
        gpos: Pos2,
        clip: Rect,
        gutter_w: f32,
        caret_line: usize,
        char_w: f32,
        font: &FontId,
    ) {
        let theme = self.theme.clone();
        let g = Rect::from_min_max(
            clip.left_top(),
            egui::pos2(clip.left() + gutter_w, clip.bottom()),
        );
        painter.rect_filled(g, 0.0, theme.gutter_bg);
        painter.line_segment(
            [g.right_top(), g.right_bottom()],
            Stroke::new(1.0, theme.gutter_fg.gamma_multiply(0.35)),
        );
        let small = FontId::monospace((font.size - 1.0).max(8.0));
        let mut worst: std::collections::HashMap<usize, (Severity, String)> = Default::default();
        if let Some(c) = &self.cache {
            for d in self
                .validation
                .diagnostics
                .iter()
                .filter(|d| self.is_shown(d))
            {
                let l = c.lines.line_of(d.span.start);
                let e = worst.entry(l).or_insert((d.severity, d.message.clone()));
                if d.severity < e.0 {
                    *e = (d.severity, d.message.clone());
                }
            }
        }
        let mut line_no = 0usize;
        for prow in galley.rows.iter() {
            let r = prow.rect().translate(gpos.to_vec2());
            let this = line_no;
            if prow.ends_with_newline {
                line_no += 1;
            }
            if r.bottom() < clip.top() {
                continue;
            }
            if r.top() > clip.bottom() {
                break;
            }
            if self.options.show_line_numbers {
                let col = if this == caret_line {
                    theme.gutter_fg_active
                } else {
                    theme.gutter_fg
                };
                painter.text(
                    egui::pos2(g.right() - 8.0, r.center().y),
                    egui::Align2::RIGHT_CENTER,
                    (this + 1).to_string(),
                    small.clone(),
                    col,
                );
            }
            if let Some((sev, msg)) = worst.get(&this) {
                let c = egui::pos2(g.left() + 8.0, r.center().y);
                painter.circle_filled(c, (char_w * 0.42).max(3.0), theme.severity_color(*sev));
                let hit = Rect::from_center_size(c, vec2(14.0, r.height()));
                let resp = ui.interact(hit, self.id.with(("gutter", this)), Sense::hover());
                resp.on_hover_text(msg.clone());
            }
        }
    }

    /// Hover cards: diagnostics and command docs under the pointer.
    #[allow(clippy::too_many_arguments)]
    fn hover_ui(
        &mut self,
        ui: &mut Ui,
        resp: &egui::Response,
        galley: &Galley,
        gpos: Pos2,
        clip: Rect,
        char_w: f32,
        now: f64,
    ) {
        let ptr = ui.input(|i| i.pointer.hover_pos());
        let moving = ui.input(|i| i.pointer.is_moving());
        let over_card = match (&self.hover, ptr) {
            (Some(h), Some(p)) => h.rect.map(|r| r.expand(6.0).contains(p)).unwrap_or(false),
            _ => false,
        };
        let mut target: Option<Span> = None;
        let mut diags = vec![];
        let mut info = None;
        let mut sound = None;
        if let (Some(p), true) = (ptr, resp.hovered() && self.completion.is_none()) {
            if clip.contains(p) && p.x > clip.left() + 40.0 {
                let off = galley.cursor_from_pos(p - gpos).index.0;
                diags = self.all_diags_at(off);
                if let Some(c) = &self.cache {
                    info = c.analysis.hover_with(off, self.strings.as_ref());
                    if let Some(lookup) = &self.sounds {
                        if let Some(a) = c.analysis.sound_arg_at(off) {
                            info = Some(crate::sounds::hover(&a, lookup));
                            sound = Some(a);
                        }
                    }
                }
                let mut spans: Vec<Span> = diags.iter().map(|d| d.span).collect();
                if let Some(i) = &info {
                    spans.push(i.span);
                }
                // the pointer must really be over the glyphs
                target = spans
                    .into_iter()
                    .find(|s| span_rect(galley, gpos, *s, char_w).expand(1.0).contains(p));
                if target.is_none() {
                    diags.clear();
                    info = None;
                    sound = None;
                }
            }
        }
        match target {
            Some(span) => {
                let same = self.hover_since.map(|(s, _)| s == span).unwrap_or(false);
                if !same {
                    self.hover_since = Some((span, now));
                    self.hover = None;
                }
                let since = self.hover_since.map(|(_, t)| t).unwrap_or(now);
                if now - since >= 0.35 {
                    let pos = p_or(ptr, span_rect(galley, gpos, span, char_w).left_bottom());
                    let keep_rect = self
                        .hover
                        .as_ref()
                        .filter(|h| h.span == span)
                        .and_then(|h| h.rect);
                    self.hover = Some(HoverCard {
                        span,
                        diags,
                        info,
                        pos: egui::pos2(
                            pos.x,
                            span_rect(galley, gpos, span, char_w).bottom() + 4.0,
                        ),
                        rect: keep_rect,
                        sound,
                    });
                } else {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
            None => {
                if !(over_card && !moving || over_card) {
                    self.hover = None;
                }
                self.hover_since = None;
            }
        }
        let mut play: Option<crate::sounds::SoundArg> = None;
        if let Some(h) = &self.hover {
            let theme = self.theme.clone();
            let area = egui::Area::new(self.id.with("hover"))
                .order(egui::Order::Tooltip)
                .fixed_pos(h.pos)
                .constrain(true)
                .interactable(true)
                .show(ui.ctx(), |ui| {
                    base_visuals(ui, &theme);
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(460.0);
                        for d in &h.diags {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(
                                    theme.severity_color(d.severity),
                                    match d.severity {
                                        Severity::Error => "error",
                                        Severity::Warning => "warning",
                                        Severity::Info => "info",
                                    },
                                );
                                ui.label(&d.message);
                                ui.weak(d.code);
                            });
                        }
                        if let Some(i) = &h.info {
                            if !h.diags.is_empty() {
                                ui.separator();
                            }
                            hover_card(ui, i, &theme);
                        }
                        if let Some(s) = &h.sound {
                            if ui
                                .button("Play")
                                .on_hover_text(
                                    "Play it in the editor (uses the open level's sound layers)",
                                )
                                .clicked()
                            {
                                play = Some(s.clone());
                            }
                        }
                    });
                });
            let r = area.response.rect;
            if let Some(h) = &mut self.hover {
                h.rect = Some(r);
            }
        }
        if let Some(s) = play {
            self.events.push(crate::ScriptEvent::PlaySound(s));
        }
    }

    fn popups(&mut self, ui: &mut Ui, galley: &Galley, gpos: Pos2, clip: Rect, row_h: f32) {
        let theme = self.theme.clone();
        let c = caret_rect(galley, gpos, self.caret.1);
        if !clip.contains(c.center()) {
            return;
        }
        // Signature help above the caret line (below it when there is no room above). Drawn as a
        // child of the editor's own Ui, on the editor's layer and clipped to the text view: a
        // separate foreground Area would float over every other egui window, including windows
        // the host raised above the editor.
        self.sig_drawn = None;
        let sig = self
            .cache
            .as_ref()
            .and_then(|c| c.analysis.signature_help(self.caret.1));
        let text_focused = ui.memory(|m| m.has_focus(self.text_id()));
        if let (Some(sig), true, true) = (sig, self.hover.is_none(), text_focused) {
            let size_id = self.id.with("sig-size");
            let size: egui::Vec2 = ui
                .data(|d| d.get_temp(size_id))
                .unwrap_or(vec2(320.0, 2.0 * row_h + 12.0));
            let mut top = c.top() - 4.0 - size.y;
            if top < clip.top() {
                top = if self.completion.is_some() {
                    f32::NAN
                } else {
                    c.bottom() + 4.0
                };
            }
            if top.is_finite() {
                let left = c.left().min(clip.right() - size.x).max(clip.left());
                let max = Rect::from_min_size(
                    egui::pos2(left, top),
                    vec2(
                        (clip.right() - left).max(120.0).min(736.0),
                        clip.bottom() - top,
                    ),
                );
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(max)
                        .layout(egui::Layout::top_down(Align::Min)),
                );
                child.set_clip_rect(clip.intersect(ui.clip_rect()));
                child.style_mut().interaction.selectable_labels = false;
                base_visuals(&mut child, &theme);
                let r = Frame::popup(child.style()).show(&mut child, |ui| {
                    ui.set_max_width(720.0);
                    let font = FontId::monospace(theme.font_size);
                    let mut job = egui::text::LayoutJob::default();
                    let label = &sig.label;
                    let active = sig.params.get(sig.active).map(|p| p.label_range);
                    let mut pos = 0;
                    let put =
                        |job: &mut egui::text::LayoutJob, s: &str, color: Color32, bold: bool| {
                            if s.is_empty() {
                                return;
                            }
                            job.append(
                                s,
                                0.0,
                                TextFormat {
                                    font_id: font.clone(),
                                    color,
                                    underline: if bold {
                                        Stroke::new(1.5, theme.accent)
                                    } else {
                                        Stroke::NONE
                                    },
                                    ..Default::default()
                                },
                            );
                        };
                    if let Some((a, b)) = active {
                        put(
                            &mut job,
                            &label[pos..a],
                            theme.text.gamma_multiply(0.8),
                            false,
                        );
                        put(&mut job, &label[a..b], theme.accent, true);
                        pos = b;
                    }
                    put(
                        &mut job,
                        &label[pos..],
                        theme.text.gamma_multiply(0.8),
                        false,
                    );
                    ui.label(job);
                    if let Some(p) = sig.params.get(sig.active) {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(RichTextHelper::accent(&p.name, theme.accent));
                            let mut extra = p.detail.clone().unwrap_or_default();
                            if p.optional && !extra.contains("optional") {
                                extra = if extra.is_empty() {
                                    "optional".into()
                                } else {
                                    format!("{extra}, optional")
                                };
                            }
                            if !extra.is_empty() {
                                ui.weak(format!("({extra})"));
                            }
                        });
                        if !p.doc.is_empty() {
                            ui.label(egui::RichText::new(p.doc.clone()).small());
                        }
                    } else if !sig.doc.is_empty() {
                        ui.label(egui::RichText::new(sig.doc.lines().next().unwrap_or("")).small());
                    }
                });
                let rect = r.response.rect;
                if (rect.size() - size).length() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(size_id, rect.size()));
                    ui.ctx().request_repaint();
                }
                self.sig_drawn = Some((child.layer_id(), rect));
            }
        }
        // completion popup below
        if let Some(st) = &self.completion {
            let items = st.items.clone();
            let sel = st.selected;
            let scroll = st.scroll_to_selected;
            let mut clicked: Option<usize> = None;
            egui::Area::new(self.id.with("completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(c.left(), c.bottom() + 2.0))
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    base_visuals(ui, &theme);
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(480.0);
                        // rows are about one text row plus padding; the list scrolls past 11 of
                        // them
                        let row_est = row_h
                            + ui.spacing().button_padding.y * 2.0
                            + ui.spacing().item_spacing.y;
                        let list_h = (items.len() as f32 * row_est)
                            .min(row_h * 11.0)
                            .max(row_est);
                        ScrollArea::vertical()
                            .max_height(list_h)
                            .min_scrolled_height(list_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (i, it) in items.iter().enumerate() {
                                    let tag = it.kind.tag();
                                    let tag_col = match it.kind {
                                        CompletionKind::Command | CompletionKind::Type => {
                                            theme.command
                                        }
                                        CompletionKind::Constant | CompletionKind::Variable => {
                                            theme.constant
                                        }
                                        CompletionKind::Snippet | CompletionKind::Hook => {
                                            theme.function
                                        }
                                        CompletionKind::Keyword => theme.keyword,
                                        CompletionKind::Field => theme.field,
                                    };
                                    let mut job = egui::text::LayoutJob::default();
                                    job.append(
                                        &format!("{tag:<6} "),
                                        0.0,
                                        TextFormat {
                                            font_id: FontId::monospace(theme.font_size - 2.0),
                                            color: tag_col,
                                            ..Default::default()
                                        },
                                    );
                                    job.append(
                                        &it.label,
                                        0.0,
                                        TextFormat {
                                            font_id: FontId::monospace(theme.font_size),
                                            color: if it.deprecated {
                                                theme.text.gamma_multiply(0.5)
                                            } else {
                                                theme.text
                                            },
                                            strikethrough: if it.deprecated {
                                                Stroke::new(1.0, theme.text.gamma_multiply(0.5))
                                            } else {
                                                Stroke::NONE
                                            },
                                            ..Default::default()
                                        },
                                    );
                                    if !it.detail.is_empty() {
                                        let mut d: String = it.detail.chars().take(52).collect();
                                        if it.detail.chars().count() > 52 {
                                            d.push_str("…");
                                        }
                                        job.append(
                                            &format!("   {d}"),
                                            0.0,
                                            TextFormat {
                                                font_id: FontId::proportional(
                                                    theme.font_size - 2.0,
                                                ),
                                                color: theme.text.gamma_multiply(0.55),
                                                ..Default::default()
                                            },
                                        );
                                    }
                                    let r = ui.selectable_label(i == sel, job);
                                    if i == sel && scroll {
                                        r.scroll_to_me(None);
                                    }
                                    if r.clicked() {
                                        clicked = Some(i);
                                    }
                                }
                            });
                    });
                });
            if let Some(s) = &mut self.completion {
                s.scroll_to_selected = false;
            }
            if let Some(i) = clicked {
                self.accept_completion(i);
            }
        }
    }
}

/// Popups live in their own areas outside the editor's scope: give them the visuals of the theme.
fn base_visuals(ui: &mut Ui, theme: &Theme) {
    if ui.visuals().dark_mode != theme.is_dark {
        *ui.visuals_mut() = if theme.is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
    }
}

fn p_or(p: Option<Pos2>, fallback: Pos2) -> Pos2 {
    p.unwrap_or(fallback)
}

pub(crate) struct RichTextHelper;
impl RichTextHelper {
    pub fn accent(s: &str, c: Color32) -> egui::RichText {
        egui::RichText::new(s).color(c)
    }
}

pub(crate) fn hover_card(ui: &mut Ui, i: &HoverInfo, theme: &Theme) {
    if let Some(t) = &i.title {
        ui.label(egui::RichText::new(t).monospace().color(theme.accent));
    }
    if !i.summary.is_empty() {
        ui.label(&i.summary);
    }
    if !i.params.is_empty() {
        ui.add_space(2.0);
        for (n, t) in &i.params {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(n).monospace().color(theme.command));
                ui.label(egui::RichText::new(t).small().weak());
            });
        }
    }
    if let Some(e) = i.examples.first() {
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(e)
                .monospace()
                .small()
                .color(theme.text.gamma_multiply(0.8)),
        );
    }
    if let Some((label, url)) = &i.link {
        ui.add_space(2.0);
        if ui.button(label).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
        }
    }
}

pub(crate) fn _unused(_: &Transaction) {}
