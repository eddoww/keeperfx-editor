//! System fallback fonts for the editor's egui contexts (the window, `--screenshot` /
//! `--actions` headless runs and the egui_kittest UI tests).
//!
//! egui's bundled fonts (Ubuntu-Light, Hack, Noto Emoji, the emoji icon font) cover Latin, Greek,
//! Cyrillic and common symbols. Game strings (17 languages, including Chinese, Japanese and
//! Korean), Workshop text, campaign names and file names can hold any character, so [`install`]
//! adds egui's own system font provider (`egui_system_font_provider`): for a character none of the
//! configured fonts has, it asks the operating system's font fallback (fontconfig on Linux,
//! DirectWrite on Windows, CoreText on macOS, through `fontique`) for an installed font that has
//! it, and appends that font to the family's fallback chain, after egui's fonts. Nothing is
//! bundled; the font files are memory-mapped when first needed.
//!
//! Cost: listing the system fonts runs once per process on a background thread started by
//! [`warm_up`] (or the first [`install`]); a character lookup waits for it only if the list is not
//! ready yet. Each family asks at most once per missing character, and a found font then covers the
//! rest of its script without further lookups.
//!
//! # Missing glyphs
//! A character that no font on the system has is drawn as a box ("tofu"): the `.notdef` glyph
//! of the family's first font (egui's default [`egui::MissingGlyphPolicy::Tofu`]). egui only
//! panics on such a character when the policy is [`egui::MissingGlyphPolicy::Panic`], which is
//! what `egui_kittest` harnesses use by default (`HarnessBuilder::allow_missing_glyphs` turns it
//! off); the application itself never sets it.
use egui_system_font_provider::SystemFontProvider;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

static PROVIDER: OnceLock<(Arc<SystemFontProvider>, Duration)> = OnceLock::new();

fn provider() -> Arc<SystemFontProvider> {
    PROVIDER
        .get_or_init(|| {
            let t0 = Instant::now();
            let p = Arc::new(SystemFontProvider::new());
            (p, t0.elapsed())
        })
        .0
        .clone()
}

/// Start listing the system fonts on a background thread now (once per process), so the list
/// is ready when the first unusual character is laid out. Cheap; call early at start-up.
pub fn warm_up() {
    provider();
}

/// How long creating the provider took on the calling thread (the font listing itself runs in
/// the background). `None` before [`warm_up`] / [`install`].
pub fn startup_cost() -> Option<Duration> {
    PROVIDER.get().map(|p| p.1)
}

/// Add the system font fallback to `ctx`, after its configured fonts. Idempotent per context.
pub fn install(ctx: &egui::Context) {
    let id = egui::Id::unique("kfx-fonts::installed");
    if ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
    ctx.add_font_provider(provider());
}

/// Names of the system fonts `ctx` has loaded so far (`system:<family>:<style>:<index>`).
pub fn discovered(ctx: &egui::Context) -> Vec<String> {
    ctx.fonts(|f| {
        f.discovered_fonts()
            .iter()
            .map(|i| i.name.clone())
            .collect()
    })
}

/// Wait until the system font list is ready and look up a font for each of `chars` once, so
/// that the timing of the first frame showing them does not include the listing. Returns the
/// time spent. For start-up measurements and tests.
pub fn resolve_now(ctx: &egui::Context, chars: &str) -> Duration {
    let t0 = Instant::now();
    let id = egui::FontId::proportional(14.0);
    ctx.fonts_mut(|f| {
        for c in chars.chars() {
            let _ = f.has_glyph(&id, c);
        }
    });
    t0.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A first frame so the fonts exist.
    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
        ctx
    }

    /// Latin / Greek / Cyrillic come from egui's own fonts; CJK, Hangul and other scripts from
    /// the system when it has a font for them. Laying out any character never panics.
    #[test]
    fn system_fallback_and_no_panic() {
        let ctx = ctx();
        install(&ctx);
        install(&ctx); // idempotent
        let t = resolve_now(&ctx, "中한あЖ★→\u{2318}\u{FFFD}");
        let id = egui::FontId::proportional(14.0);
        let mut have = vec![];
        for c in "AäЖωé".chars() {
            assert!(
                ctx.fonts_mut(|f| f.has_glyph(&id, c)),
                "{c:?} is in egui's own fonts"
            );
        }
        for c in "中한あ★→\u{2318}\u{FFFD}\u{E000}".chars() {
            have.push((c, ctx.fonts_mut(|f| f.has_glyph(&id, c))));
        }
        eprintln!(
            "provider created in {:?}, first lookups {t:?}; glyphs: {have:?}; fonts: {:?}",
            startup_cost(),
            discovered(&ctx)
        );
        // laying out (and painting) text no font has: tofu, not a panic
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.label("中文 日本語 한국어 Ж ★ \u{E000} \u{10FFFD}");
        });
        out.textures_delta.clear();
    }
}
