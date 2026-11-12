//! Sound arguments of DK script commands: hover with what the game would play, and a "Play" button
//! that asks the host to play it ([`crate::ScriptEvent::PlaySound`]).
//!
//! * `PLAY_MESSAGE(player, SPEECH|SOUND, n)`: a number is a speech message index (`SPEECH`,
//!   `output_message`) or a sound effect id (`SOUND`, `play_non_3d_sample`); a text is a speech
//!   file (`output_message_from_path`) or a sound name / file (`sound_manager_load_named_sound`)
//!   (`play_message_check`, `src/lvl_script_commands.c:5018`; `script_play_message`,
//!   `src/gui_soundmsgs.cpp:563`; `msgtype_desc`, `src/lvl_script_commands.c:196`).
//! * `SET_MUSIC(track)`: a music track number or file.
//!
//! The host supplies a [`SoundLookup`] that describes what an argument resolves to for the open
//! level (sounds.cfg layers, banks, custom files); without one there is no hover and no button.
use crate::lang::HoverInfo;
use kfx_script::services::{context_at, Context};
use kfx_script::{Document, Span};
use std::sync::Arc;

/// What kind of sound an argument names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundArgKind {
    /// A speech message index or a speech file.
    Speech,
    /// A sound effect id, a sound name or a sound file.
    Sound,
    /// A music track number or file.
    Music,
}

/// A sound argument of a script command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundArg {
    pub command: String,
    pub kind: SoundArgKind,
    /// The argument as the game reads it (strings without quotes).
    pub value: String,
    /// Where the argument is written.
    pub span: Span,
}

impl SoundArg {
    /// The value when it is a plain non-negative integer.
    pub fn number(&self) -> Option<u32> {
        let v = self.value.trim();
        (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            .then(|| v.parse().ok())
            .flatten()
    }
}

/// Describes what a sound argument plays for the open level (`None`: nothing to say).
pub type SoundLookup = Arc<dyn Fn(&SoundArg) -> Option<String> + Send + Sync>;

/// The sound argument under `offset`, if any.
pub fn sound_arg_at(doc: &Document, offset: usize) -> Option<SoundArg> {
    let Context::Arg { cmd, slot, .. } = context_at(doc, offset) else {
        return None;
    };
    let arg = |s: usize| cmd.args.iter().find(|a| a.slot == s && a.func.is_none());
    let (kind, at) = match cmd.name.to_ascii_uppercase().as_str() {
        "PLAY_MESSAGE" if slot == 2 => {
            let t = arg(1)?.text.to_ascii_uppercase();
            let k = match t.as_str() {
                "SPEECH" => SoundArgKind::Speech,
                "SOUND" => SoundArgKind::Sound,
                _ => return None,
            };
            (k, arg(2)?)
        }
        "SET_MUSIC" if slot == 0 => (SoundArgKind::Music, arg(0)?),
        _ => return None,
    };
    if !at.span.touches(offset) || at.text.trim().is_empty() {
        return None;
    }
    Some(SoundArg {
        command: cmd.name.clone(),
        kind,
        value: at.text.clone(),
        span: at.text_span,
    })
}

/// Hover card for a sound argument.
pub fn hover(arg: &SoundArg, lookup: &SoundLookup) -> HoverInfo {
    let what = match arg.kind {
        SoundArgKind::Speech => "speech",
        SoundArgKind::Sound => "sound effect",
        SoundArgKind::Music => "music",
    };
    let summary =
        lookup(arg).unwrap_or_else(|| format!("No {what} '{}' found for this level.", arg.value));
    HoverInfo {
        span: arg.span,
        title: Some(format!("{}: {what} {}", arg.command, arg.value)),
        summary,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, needle: &str) -> Option<SoundArg> {
        // PLAY_MESSAGE and SET_MUSIC are KeeperFX commands: level version 1
        let src = format!("LEVEL_VERSION(1)\n{src}");
        let doc = Document::parse(src.as_bytes());
        let off = src.rfind(needle).unwrap() + 1;
        sound_arg_at(&doc, off)
    }

    #[test]
    fn play_message_and_set_music() {
        let s = at("PLAY_MESSAGE(PLAYER0,SPEECH,57)\n", "57").unwrap();
        assert_eq!(
            (s.kind, s.value.as_str(), s.number()),
            (SoundArgKind::Speech, "57", Some(57))
        );
        let s = at("PLAY_MESSAGE(PLAYER0,SOUND,\"custom.wav\")\n", "custom").unwrap();
        assert_eq!(
            (s.kind, s.value.as_str(), s.number()),
            (SoundArgKind::Sound, "custom.wav", None)
        );
        assert!(at("PLAY_MESSAGE(PLAYER0,SPEECH,57)\n", "PLAYER0").is_none());
        let s = at("SET_MUSIC(3)\n", "3").unwrap();
        assert_eq!((s.kind, s.number()), (SoundArgKind::Music, Some(3)));
        assert!(at("ADD_GOLD_TO_PLAYER(PLAYER0,57)\n", "57").is_none());
    }

    #[test]
    fn hover_through_the_dk_analysis() {
        use crate::Language;
        let src = "LEVEL_VERSION(1)\nPLAY_MESSAGE(PLAYER0,SOUND,32)\n";
        let an = crate::DkLanguage.analyze(src.as_bytes());
        let a = an
            .sound_arg_at(src.rfind("32").unwrap() + 1)
            .expect("a sound argument");
        let lookup: SoundLookup =
            Arc::new(|a: &SoundArg| (a.number() == Some(32)).then(|| "payday1.wav".to_string()));
        let h = hover(&a, &lookup);
        assert_eq!(h.summary, "payday1.wav");
        assert_eq!(h.title.as_deref(), Some("PLAY_MESSAGE: sound effect 32"));
        assert_eq!(&src.as_bytes()[h.span.start..h.span.end], b"32");
    }
}
