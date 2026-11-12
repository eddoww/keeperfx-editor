//! Recording mode of the headless renderer (`--record`): the editor is driven frame by frame
//! with simulated input (pointer moves, button presses, key presses, typed text) and every
//! frame is written out, for tutorial and demonstration videos.
//!
//! Everything that happens on screen is the editor reacting to the simulated input or to the
//! scripted actions; the recorder only adds a drawn pointer (an arrow at the simulated pointer
//! position, with a ring for each click) so that viewers can follow the input, and captions
//! are not drawn at all: they are written to a side file with their frame numbers for the
//! video encoder.
//!
//! Recording actions (in the `--actions` list; without `--record` they still apply their input,
//! but no frame is written):
//!
//! | Action | Does |
//! |---|---|
//! | `{"hold": 1.5}` | render 1.5 s of frames (the UI keeps running: toasts fade, liquids move); add `"realtime": true` to pace the frames by the wall clock (a game connected through the live link runs in real time); `"write": false` runs the frames without writing them |
//! | `{"rec": 3}` | render 3 frames |
//! | `{"auto_frames": 1}` | frames rendered after every other action (default 0) |
//! | `{"stride": 4}` | write only every 4th frame (time-lapse; the UI still runs every frame) |
//! | `{"cursor": false}` | hide / show the drawn pointer |
//! | `{"view_only": true}` | record only the 3D view at the full frame size, without the UI |
//! | `{"anim": true}` | animate liquids and flames with the frame count (otherwise frame `--frame`) |
//! | `{"move": {"to": T, "secs": 0.6, "ease": "inout"}}` | move the pointer to target `T` |
//! | `{"tap": "left"}`, `{"tap": {"button": "left", "to": T, "secs": 0.5, "count": 2}}` | move (optionally) and click |
//! | `{"down": "left"}`, `{"up": "left"}` | press / release a button where the pointer is |
//! | `{"drag": ...}` is the slab drag of the action list; pointer drags: `{"pdrag": {"to": T, "secs": 1, "button": "right"}}` |
//! | `{"stroke": {"slabs": [[x,y], ...], "secs": 2, "button": "left", "two_d": false}}` | press on the first slab, drag through the others, release |
//! | `{"type": {"text": "...", "cps": 14}}` | type text (Enter for a new line) into the focused widget |
//! | `{"keys": "ctrl+z", "times": 3, "secs": 0.3}` | key presses with modifiers (`ctrl`, `shift`, `alt`) |
//! | `{"wheel": {"dy": -400, "secs": 0.8}}` | scroll the wheel where the pointer is (points, spread over the time) |
//! | `{"camera_to": {"cam": [x,y,zoom,yaw,pitch], "secs": 2, "ease": "inout"}}` | move the 3D camera (no input) |
//! | `{"orbit": {"deg": 360, "secs": 20}}` | turn the 3D camera around its target |
//! | `{"fly_path": {"move": [r,u,f], "look": [yaw,pitch], "secs": 4}}` | move the fly view by this much over the time |
//! | `{"caption": "text"}`, `{"caption": null}` | start / end a caption at the current frame |
//!
//! A target `T` is a pixel `[x, y]` (screen pixels), `{"slab": [x, y]}` or `{"stl": [x, y]}` on
//! the map (add `"two_d": true` for the 2D pane), or `{"text": "Split", "nth": 0, "in": [x0, y0,
//! x1, y1]}`: the centre of the n-th text drawn last frame that equals the given text (within the
//! pixel rectangle `in`), or `{"word": "ADD_PARTY_TO_LEVEL"}`: a word inside any drawn text (a
//! code editor draws its text as one block); add `"dx"`/`"dy"` (pixels) to move from the centre.
use eframe::egui;
use egui::{Pos2, Rect};
use std::io::Write;
use std::path::PathBuf;

/// Where the frames go.
#[derive(Debug, Clone, PartialEq)]
pub enum Sink {
    /// Numbered PNG files `frame_000000.png` in a folder.
    Dir(PathBuf),
    /// Raw RGBA frames on standard output (pipe into `ffmpeg -f rawvideo -pix_fmt rgba`).
    Stdout,
}

impl Sink {
    pub fn parse(s: &str) -> Sink {
        if s == "-" {
            Sink::Stdout
        } else {
            Sink::Dir(PathBuf::from(s))
        }
    }
}

/// A caption with its first and last frame (written frames).
#[derive(Debug, Clone, PartialEq)]
pub struct Caption {
    pub start: u64,
    pub end: Option<u64>,
    pub text: String,
}

/// Easing of a movement over `t` in 0..1.
pub fn ease(kind: &str, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        "linear" => t,
        "in" => t * t * t,
        "out" => 1.0 - (1.0 - t).powi(3),
        // smootherstep: zero velocity and acceleration at both ends
        _ => t * t * t * (t * (t * 6.0 - 15.0) + 10.0),
    }
}

/// Number of frames for `secs` seconds at `fps` (at least one).
pub fn frames_for(secs: f32, fps: f32) -> u32 {
    ((secs * fps).round() as i64).max(1) as u32
}

/// The point at `t` (0..1, by length) along a polyline.
pub fn along(points: &[Pos2], t: f32) -> Pos2 {
    match points.len() {
        0 => Pos2::ZERO,
        1 => points[0],
        _ => {
            let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
            if total <= 0.0 {
                return points[0];
            }
            let mut want = t.clamp(0.0, 1.0) * total;
            for w in points.windows(2) {
                let d = w[0].distance(w[1]);
                if want <= d && d > 0.0 {
                    return w[0] + (w[1] - w[0]) * (want / d);
                }
                want -= d;
            }
            *points.last().unwrap()
        }
    }
}

/// Parse "ctrl+shift+z" into modifiers and a key.
pub fn parse_keys(s: &str) -> Result<(egui::Modifiers, egui::Key), String> {
    let mut m = egui::Modifiers::NONE;
    let mut key = None;
    for part in s.split('+').map(|p| p.trim()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => {
                m.ctrl = true;
                m.command = true;
            }
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            "" => return Err(format!("bad key {s:?}")),
            k => {
                let name = match k {
                    "esc" => "Escape".to_string(),
                    "del" => "Delete".to_string(),
                    "pgup" => "PageUp".to_string(),
                    "pgdn" => "PageDown".to_string(),
                    "up" => "ArrowUp".to_string(),
                    "down" => "ArrowDown".to_string(),
                    "left" => "ArrowLeft".to_string(),
                    "right" => "ArrowRight".to_string(),
                    other => {
                        let mut c = other.chars();
                        c.next()
                            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                            .unwrap_or_default()
                    }
                };
                key = Some(
                    egui::Key::from_name(&name).ok_or_else(|| format!("unknown key {part:?}"))?,
                );
            }
        }
    }
    Ok((m, key.ok_or_else(|| format!("no key in {s:?}"))?))
}

pub fn parse_button(s: &str) -> Result<egui::PointerButton, String> {
    Ok(match s {
        "left" | "primary" => egui::PointerButton::Primary,
        "right" | "secondary" => egui::PointerButton::Secondary,
        "middle" => egui::PointerButton::Middle,
        x => return Err(format!("unknown button {x:?}")),
    })
}

/// A text drawn in a frame: its string, screen rectangle (points), and the laid out galley with
/// its position and clip rectangle (to find words inside a long text such as a code editor).
#[derive(Clone)]
pub struct DrawnText {
    pub text: String,
    pub rect: Rect,
    pub pos: Pos2,
    pub clip: Rect,
    pub galley: std::sync::Arc<egui::Galley>,
}

/// Texts drawn in a frame.
pub fn collect_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<DrawnText>) {
    fn walk(s: &egui::Shape, clip: Rect, out: &mut Vec<DrawnText>) {
        match s {
            egui::Shape::Text(t) => {
                let r = t.galley.rect.translate(t.pos.to_vec2());
                if clip.intersects(r) {
                    out.push(DrawnText {
                        text: t.galley.text().to_string(),
                        rect: r,
                        pos: t.pos,
                        clip,
                        galley: t.galley.clone(),
                    });
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    for c in shapes {
        walk(&c.shape, c.clip_rect, out);
    }
}

/// The screen rectangle of the `nth` visible occurrence of `word` inside any drawn text (for
/// example a word of a code editor, which is drawn as one text), within `within` when given.
pub fn find_word(
    texts: &[DrawnText],
    word: &str,
    nth: usize,
    within: Option<Rect>,
) -> Option<Rect> {
    if word.is_empty() {
        return None;
    }
    let mut found = vec![];
    for t in texts {
        let mut from = 0;
        while let Some(i) = t.text[from..].find(word) {
            let b = from + i;
            let c0 = t.text[..b].chars().count();
            let c1 = c0 + word.chars().count();
            let a = t.galley.pos_from_cursor(egui::text::CCursor::new(c0));
            let z = t.galley.pos_from_cursor(egui::text::CCursor::new(c1));
            let r = Rect::from_min_max(a.min, egui::pos2(z.max.x.max(a.max.x), a.max.y))
                .translate(t.pos.to_vec2());
            if t.clip.contains(r.center()) && within.is_none_or(|w| w.contains(r.center())) {
                found.push(r);
            }
            from = b + word.len();
        }
    }
    found.get(nth).copied()
}

/// The first forbidden text (case-insensitive) found in the texts of a frame.
pub fn forbidden<'a>(texts: &[DrawnText], forbid: &'a [String]) -> Option<(&'a str, String)> {
    for DrawnText { text: t, .. } in texts {
        let low = t.to_lowercase();
        for f in forbid {
            if !f.is_empty() && low.contains(&f.to_lowercase()) {
                return Some((f.as_str(), t.clone()));
            }
        }
    }
    None
}

/// Find the `nth` text equal to `want` (trimmed; or, failing that, starting with it), inside
/// `within` when given; its rectangle.
pub fn find_text(
    texts: &[DrawnText],
    want: &str,
    nth: usize,
    within: Option<Rect>,
) -> Option<Rect> {
    let ok = |r: &Rect| within.is_none_or(|w| w.contains(r.center()));
    // labels may start with an icon glyph ("🗋 New")
    let plain = |t: &str| {
        t.trim()
            .trim_start_matches(|c: char| !c.is_ascii())
            .trim()
            .to_string()
    };
    let exact: Vec<Rect> = texts
        .iter()
        .filter(|t| plain(&t.text) == want && ok(&t.rect))
        .map(|t| t.rect)
        .collect();
    if let Some(r) = exact.get(nth) {
        return Some(*r);
    }
    let prefix: Vec<Rect> = texts
        .iter()
        .filter(|t| plain(&t.text) != want && plain(&t.text).starts_with(want) && ok(&t.rect))
        .map(|t| t.rect)
        .collect();
    prefix.get(nth.saturating_sub(exact.len())).copied()
}

/// The drawn pointer: an arrow with its tip at `p`, and a fading ring for every recent click.
pub fn paint_cursor(
    painter: &egui::Painter,
    p: Pos2,
    pressed: bool,
    ripples: &[(Pos2, f64)],
    now: f64,
) {
    use egui::{Color32, Stroke};
    for (at, t0) in ripples {
        let age = (now - t0) as f32;
        if (0.0..0.6).contains(&age) {
            let k = age / 0.6;
            let a = (1.0 - k).powf(1.5);
            painter.circle_stroke(
                *at,
                6.0 + 22.0 * k,
                Stroke::new(
                    3.0 * (1.0 - k) + 1.0,
                    Color32::from_rgba_unmultiplied(255, 214, 120, (230.0 * a) as u8),
                ),
            );
        }
    }
    // a classic arrow, 22 points tall
    let s = 1.15;
    let pts: Vec<Pos2> = [
        (0.0, 0.0),
        (0.0, 17.0),
        (4.2, 13.0),
        (7.4, 19.6),
        (10.2, 18.4),
        (7.1, 11.9),
        (12.6, 11.9),
    ]
    .iter()
    .map(|(x, y)| p + egui::vec2(x * s, y * s))
    .collect();
    let shadow: Vec<Pos2> = pts.iter().map(|q| *q + egui::vec2(1.5, 2.0)).collect();
    painter.add(egui::Shape::convex_polygon(
        shadow[..3].to_vec(),
        Color32::from_black_alpha(70),
        Stroke::NONE,
    ));
    painter.add(egui::Shape::closed_line(
        shadow,
        Stroke::new(3.0, Color32::from_black_alpha(60)),
    ));
    let fill = if pressed {
        Color32::from_rgb(255, 226, 150)
    } else {
        Color32::WHITE
    };
    // the arrow is concave: fill it as two convex parts
    painter.add(egui::Shape::convex_polygon(
        vec![pts[0], pts[1], pts[2], pts[5], pts[6]],
        fill,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![pts[2], pts[3], pts[4], pts[5]],
        fill,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::closed_line(
        pts,
        Stroke::new(1.3, Color32::BLACK),
    ));
}

/// State of a recording.
pub struct Recorder {
    pub sink: Option<Sink>,
    pub fps: f32,
    /// Write every n-th simulated frame.
    pub stride: u32,
    /// Simulated frames so far.
    pub sim: u64,
    /// Written frames so far.
    pub written: u64,
    /// Frames after every action that has none of its own.
    pub auto_frames: u32,
    pub cursor: bool,
    /// Record only the 3D view at the full frame size (no UI).
    pub view_only: bool,
    /// Frames are simulated but not written (`"hold"` with `"write": false`).
    pub paused: bool,
    /// Animate liquids with the frame count.
    pub anim: bool,
    pub pointer: Pos2,
    pub held: Vec<egui::PointerButton>,
    pub modifiers: egui::Modifiers,
    pub ripples: Vec<(Pos2, f64)>,
    pub captions: Vec<Caption>,
    /// Texts of the last frame (for text targets).
    pub texts: Vec<DrawnText>,
    /// Texts no frame may show (privacy check); a frame that shows one fails the run.
    pub forbid: Vec<String>,
    /// File of the captions (JSON), written at the end.
    pub captions_file: Option<PathBuf>,
}

impl Recorder {
    pub fn new(sink: Option<Sink>, fps: f32) -> Recorder {
        Recorder {
            sink,
            fps: fps.max(1.0),
            stride: 1,
            sim: 0,
            written: 0,
            auto_frames: 0,
            cursor: true,
            view_only: false,
            paused: false,
            anim: false,
            pointer: egui::pos2(-100.0, -100.0),
            held: vec![],
            modifiers: egui::Modifiers::NONE,
            ripples: vec![],
            captions: vec![],
            texts: vec![],
            forbid: vec![],
            captions_file: None,
        }
    }

    /// Whether the next simulated frame is written.
    pub fn writes_next(&self) -> bool {
        self.sink.is_some() && !self.paused && self.sim % self.stride.max(1) as u64 == 0
    }

    /// Write one RGBA frame.
    pub fn write(&mut self, w: u32, h: u32, data: Vec<u8>) -> Result<(), String> {
        match &self.sink {
            None => {}
            Some(Sink::Stdout) => {
                let mut o = std::io::stdout().lock();
                o.write_all(&data).map_err(|e| format!("stdout: {e}"))?;
            }
            Some(Sink::Dir(d)) => {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                let img = kfx_gfx::Rgba { w, h, data };
                let png = img.to_png().map_err(|e| e.to_string())?;
                let p = d.join(format!("frame_{:06}.png", self.written));
                std::fs::write(&p, png).map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
        self.written += 1;
        Ok(())
    }

    pub fn start_caption(&mut self, text: Option<&str>) {
        let at = self.written;
        if let Some(c) = self.captions.last_mut().filter(|c| c.end.is_none()) {
            c.end = Some(at);
        }
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            self.captions.push(Caption {
                start: at,
                end: None,
                text: t.to_string(),
            });
        }
    }

    /// The captions as JSON (frame numbers of the written frames, and the frame rate).
    pub fn captions_json(&self) -> serde_json::Value {
        serde_json::json!({
            "fps": self.fps,
            "frames": self.written,
            "captions": self.captions.iter().map(|c| serde_json::json!({"start": c.start, "end": c.end.unwrap_or(self.written), "text": c.text})).collect::<Vec<_>>(),
        })
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.start_caption(None);
        if let Some(p) = &self.captions_file {
            std::fs::write(
                p,
                serde_json::to_string_pretty(&self.captions_json()).unwrap_or_default(),
            )
            .map_err(|e| format!("{}: {e}", p.display()))?;
        }
        if let Some(Sink::Stdout) = &self.sink {
            std::io::stdout().flush().map_err(|e| e.to_string())?;
        }
        eprintln!(
            "record: {} frames written ({} simulated)",
            self.written, self.sim
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_ends_and_monotonic() {
        for k in ["linear", "in", "out", "inout"] {
            assert!(ease(k, 0.0).abs() < 1e-6, "{k}");
            assert!((ease(k, 1.0) - 1.0).abs() < 1e-6, "{k}");
            let mut last = -1.0;
            for i in 0..=20 {
                let v = ease(k, i as f32 / 20.0);
                assert!(v >= last, "{k} not monotonic");
                last = v;
            }
        }
        assert_eq!(ease("inout", 0.5), 0.5);
        assert_eq!(frames_for(0.0, 30.0), 1);
        assert_eq!(frames_for(2.0, 30.0), 60);
    }

    #[test]
    fn polyline_by_length() {
        let pts = [
            egui::pos2(0.0, 0.0),
            egui::pos2(10.0, 0.0),
            egui::pos2(10.0, 30.0),
        ];
        assert_eq!(along(&pts, 0.0), egui::pos2(0.0, 0.0));
        assert_eq!(along(&pts, 0.25), egui::pos2(10.0, 0.0));
        assert_eq!(along(&pts, 0.5), egui::pos2(10.0, 10.0));
        assert_eq!(along(&pts, 1.0), egui::pos2(10.0, 30.0));
        assert_eq!(along(&pts[..1], 0.7), egui::pos2(0.0, 0.0));
    }

    #[test]
    fn keys_and_buttons() {
        let (m, k) = parse_keys("ctrl+shift+z").unwrap();
        assert!(m.ctrl && m.command && m.shift && !m.alt);
        assert_eq!(k, egui::Key::Z);
        assert_eq!(parse_keys("Enter").unwrap().1, egui::Key::Enter);
        assert_eq!(parse_keys("esc").unwrap().1, egui::Key::Escape);
        assert_eq!(parse_keys("ctrl+space").unwrap().1, egui::Key::Space);
        assert!(parse_keys("ctrl+").is_err());
        assert!(parse_keys("ctrl+nokey").is_err());
        assert_eq!(
            parse_button("right").unwrap(),
            egui::PointerButton::Secondary
        );
        assert!(parse_button("foot").is_err());
    }

    #[test]
    fn text_targets_and_privacy_check() {
        let r = |x: f32| Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(20.0, 10.0));
        let ctx = egui::Context::default();
        let mut texts = vec![];
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for (s, x) in [
                ("Split", 0.0),
                ("\u{1F5CB} Split ", 100.0),
                ("Split view", 200.0),
                ("saved /srv/maps/x", 300.0),
            ] {
                let galley = ui.painter().layout_no_wrap(
                    s.to_string(),
                    egui::FontId::monospace(10.0),
                    egui::Color32::WHITE,
                );
                texts.push(DrawnText {
                    text: s.to_string(),
                    rect: r(x),
                    pos: egui::pos2(x, 0.0),
                    clip: Rect::EVERYTHING,
                    galley,
                });
            }
        });
        out.textures_delta.clear();
        assert_eq!(find_text(&texts, "Split", 0, None), Some(r(0.0)));
        assert_eq!(find_text(&texts, "Split", 1, None), Some(r(100.0)));
        assert_eq!(find_text(&texts, "Split", 2, None), Some(r(200.0)));
        assert_eq!(
            find_text(
                &texts,
                "Split",
                0,
                Some(Rect::from_min_max(
                    egui::pos2(90.0, -5.0),
                    egui::pos2(400.0, 50.0)
                ))
            ),
            Some(r(100.0))
        );
        assert_eq!(find_text(&texts, "Nothing", 0, None), None);
        let forbid = vec!["/SRV/MAPS/".to_string()];
        assert_eq!(forbidden(&texts, &forbid).map(|f| f.0), Some("/SRV/MAPS/"));
        assert!(forbidden(&texts[..3], &forbid).is_none());
        // a word inside a longer text: "view" of "Split view" starts 6 monospace cells in
        let w = find_word(&texts, "view", 0, None).unwrap();
        let cell = texts[2].galley.rect.width() / 10.0;
        assert!((w.min.x - (200.0 + 6.0 * cell)).abs() < 0.5, "{w:?}");
        assert!((w.width() - 4.0 * cell).abs() < 0.5, "{w:?}");
        assert_eq!(
            find_word(&texts, "Split", 2, None).map(|r| r.min.x),
            Some(200.0)
        );
        assert!(find_word(&texts, "nowhere", 0, None).is_none());
    }

    #[test]
    fn captions_get_frame_ranges() {
        let mut r = Recorder::new(None, 30.0);
        r.start_caption(Some("one"));
        r.written = 30;
        r.start_caption(Some("two"));
        r.written = 45;
        r.start_caption(None);
        r.written = 60;
        r.start_caption(Some("three"));
        r.written = 90;
        r.finish().unwrap();
        let j = r.captions_json();
        let c = j["captions"].as_array().unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(
            (c[0]["start"].as_u64(), c[0]["end"].as_u64()),
            (Some(0), Some(30))
        );
        assert_eq!(
            (c[1]["start"].as_u64(), c[1]["end"].as_u64()),
            (Some(30), Some(45))
        );
        assert_eq!(
            (c[2]["start"].as_u64(), c[2]["end"].as_u64()),
            (Some(60), Some(90))
        );
    }

    #[test]
    fn stride_and_pause_select_the_written_frames() {
        let mut r = Recorder::new(
            Some(Sink::Dir(
                std::env::temp_dir().join("kfx-record-test-unused"),
            )),
            30.0,
        );
        r.stride = 3;
        let w: Vec<bool> = (0..7)
            .map(|i| {
                r.sim = i;
                r.writes_next()
            })
            .collect();
        assert_eq!(w, [true, false, false, true, false, false, true]);
        r.stride = 1;
        r.paused = true;
        assert!(!r.writes_next());
        r.paused = false;
        assert!(r.writes_next());
        assert!(!Recorder::new(None, 30.0).writes_next());
    }
}
