//! Resolution with the layering of the game, on a private temp layout (no game data needed).
use kfx_audio::bank::{BankKind, SoundBank};
use kfx_audio::context::{expand_numbered, ModTiers};
use kfx_audio::{Audio, Context, LayerKind, Pcm, Source};
use std::path::{Path, PathBuf};

/// Build a sound bank in the on-disk format (see `kfx_audio::bank`): n samples `s<i>.wav`, sample i
/// is `i * 100` frames of 16-bit mono.
fn mini_bank(kind: BankKind, n: usize) -> SoundBank {
    let mut data = vec![];
    let mut dir = vec![];
    // record 0: the dummy
    dir.extend([0u8; 18]);
    dir.extend(0u32.to_le_bytes());
    dir.extend(0u32.to_le_bytes());
    dir.extend(0u32.to_le_bytes());
    dir.extend([0u8, 0]);
    for i in 1..=n {
        let frames = i * 100;
        let pcm = Pcm::new(22050, 1, vec![0.25; frames]);
        let wav = pcm.to_wav_bytes();
        let mut name = format!("s{i}.wav").into_bytes();
        name.resize(18, 0);
        dir.extend(&name);
        dir.extend((data.len() as u32).to_le_bytes());
        dir.extend(0u32.to_le_bytes());
        dir.extend((wav.len() as u32).to_le_bytes());
        dir.extend([0u8, 0]);
        data.extend(&wav);
    }
    let dir_off = data.len() as u32;
    let mut file = data;
    file.extend(&dir);
    let head_off = file.len() as u32;
    file.extend([0u8; 14]);
    file.extend(0u32.to_le_bytes());
    for e in 0..9 {
        if e == 2 {
            file.extend(dir_off.to_le_bytes());
            file.extend(0u32.to_le_bytes());
            file.extend((dir.len() as u32).to_le_bytes());
            file.extend(dir_off.to_le_bytes());
        } else {
            file.extend([0xFFu8; 16]);
        }
    }
    file.extend(head_off.to_le_bytes());
    SoundBank::from_bytes(kind, Path::new("mini.dat"), file).unwrap()
}

struct Layout {
    root: PathBuf,
}

impl Layout {
    fn new(name: &str) -> Layout {
        let root = kfx_formats::testutil::scratch_dir(&format!("kfx-audio/test-resolve-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Layout { root }
    }
    fn write(&self, rel: &str, content: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    /// A WAV file whose length (in frames) identifies it.
    fn wav(&self, rel: &str, frames: usize) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, Pcm::new(8000, 1, vec![0.1; frames]).to_wav_bytes()).unwrap();
    }
    fn zip(&self, rel: &str, entries: &[(&str, Vec<u8>)]) {
        use std::io::Write;
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(p).unwrap());
        for (n, d) in entries {
            z.start_file(*n, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(d).unwrap();
        }
        z.finish().unwrap();
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn frames_of(a: &Audio, s: &Source) -> usize {
    a.decode(s).unwrap().frames()
}

fn file_name(s: &Source) -> String {
    match s {
        Source::File(p) => p.file_name().unwrap().to_string_lossy().to_string(),
        Source::ZipEntry { entry, .. } => entry.clone(),
        other => format!("{other:?}"),
    }
}

#[test]
fn layering_of_sounds_cfg() {
    let l = Layout::new("layers");
    l.write(
        "fxdata/sounds.cfg",
        "[system]\nSpeechQueueLimit = 4\n[sounds]\nGOLD_PICKUP = 3 3\nTAB_CLICK = 5\nBUTTON = \
            7\nSPLASH = custom/splash\nMULTI = custom/hit1.wav 3\n",
    );
    l.wav("custom/splash.wav", 11);
    l.wav("fxdata/custom/splash.wav", 12); // later in the search order
    for i in 1..=3 {
        l.wav(&format!("custom/hit{i}.wav"), 20 + i);
    }
    l.write(
        "campgns/test.cfg",
        "[common]\nNAME = Test\nLEVELS_LOCATION = \
        levels/test\nCONFIGS_LOCATION = levels/test_cfgs\nCREATURES_LOCATION = \
        levels/test_crtr\nMEDIA_LOCATION = ldata\n",
    );
    l.write("levels/test_cfgs/sounds.cfg", "[sounds]\nTAB_CLICK = 6\n");
    l.write("levels/test/map00001.sounds.cfg", "[sounds]\nBUTTON = 8\n");
    l.write(
        "mods/load_order.cfg",
        "[after_base]\nmoda\n[after_campaign]\n[after_map]\n",
    );
    std::fs::create_dir_all(l.root.join("mods/moda/fxdata")).unwrap();
    l.write(
        "mods/moda/fxdata/sounds.cfg",
        "[sounds]\nTAB_CLICK = 4\nGOLD_PICKUP = 1 2\n",
    );

    let fx = mini_bank(BankKind::Effects, 20);
    let mk = |level: Option<u32>, camp: bool, mods: bool| {
        let mut ctx = Context::new(&l.root, "eng");
        if !mods {
            ctx = ctx.with_mods(ModTiers::default());
        }
        if camp {
            ctx = ctx.with_campaign(&l.root.join("campgns/test.cfg")).unwrap();
        }
        if let Some(n) = level {
            ctx = ctx.with_level(n);
        }
        Audio::with_banks(ctx, Some(mini_bank(BankKind::Effects, 20)), None)
    };
    let idx = |r: &kfx_audio::Resolved| match &r.variants[0].source {
        Source::Bank { index, .. } => *index,
        s => panic!("{s:?}"),
    };

    // 1. base only
    let a = mk(None, false, false);
    assert_eq!(idx(&a.resolve("TAB_CLICK").unwrap()), 5);
    assert_eq!(
        idx(&a.resolve("tab_click").unwrap()),
        5,
        "names are case-insensitive"
    );
    let g = a.resolve("GOLD_PICKUP").unwrap();
    assert_eq!(
        (g.variants.len(), g.declared_count, g.layer.kind),
        (3, 3, LayerKind::Install)
    );
    assert_eq!(
        g.variants
            .iter()
            .map(|v| match v.source {
                Source::Bank { index, .. } => index,
                _ => 0,
            })
            .collect::<Vec<_>>(),
        vec![3, 4, 5]
    );
    assert_eq!(idx(&a.resolve("12").unwrap()), 12);
    assert!(a.resolve("21").is_err(), "outside the bank");
    assert!(a.resolve("0").is_err());
    assert!(a.resolve("NOPE").is_err());
    assert_eq!(
        a.resolve("s7.wav").unwrap().note.as_deref(),
        Some("bank file name")
    );
    // custom file: the .wav candidate is searched in the install root first? no: fxdata comes
    // before the root
    let s = a.resolve("SPLASH").unwrap();
    assert_eq!(
        frames_of(&a, &s.variants[0].source),
        12,
        "fxdata/ is searched before the install root: {:?}",
        s.variants[0]
    );
    let m = a.resolve("MULTI").unwrap();
    assert_eq!((m.variants.len(), m.declared_count), (3, 3));
    assert_eq!(
        m.variants
            .iter()
            .map(|v| file_name(&v.source))
            .collect::<Vec<_>>(),
        ["hit1.wav", "hit2.wav", "hit3.wav"]
    );

    // 2. with the after_base mod: it is later, so it wins numeric names
    let a = mk(None, false, true);
    assert_eq!(idx(&a.resolve("TAB_CLICK").unwrap()), 4);
    assert_eq!(
        a.resolve("TAB_CLICK").unwrap().layer.kind,
        LayerKind::ModAfterBase
    );
    assert_eq!(a.resolve("GOLD_PICKUP").unwrap().variants.len(), 2);
    assert_eq!(idx(&a.resolve("BUTTON").unwrap()), 7);

    // 3. campaign over mod over base
    let a = mk(None, true, true);
    assert_eq!(idx(&a.resolve("TAB_CLICK").unwrap()), 6);
    assert_eq!(
        a.resolve("TAB_CLICK").unwrap().layer.kind,
        LayerKind::Campaign
    );

    // 4. level file on top, only when the level is selected
    assert_eq!(idx(&mk(None, true, true).resolve("BUTTON").unwrap()), 7);
    let a = mk(Some(1), true, true);
    assert_eq!(idx(&a.resolve("BUTTON").unwrap()), 8);
    assert_eq!(a.resolve("BUTTON").unwrap().layer.kind, LayerKind::Level);
    assert_eq!(idx(&a.resolve("TAB_CLICK").unwrap()), 6);
    let _ = fx;
}

#[test]
fn raw_id_redirect_custom_priority_and_quirks() {
    let l = Layout::new("raw");
    l.write(
        "fxdata/sounds.cfg",
        "[sounds]\n\
         36 = custom/boom.wav 2 MY_BOOM\n\
         9 = custom/step.wav\n\
         NAME = 5\n\
         NAME = custom/override.wav\n\
         DUP = custom/first.wav\n\
         DUP = custom/second.wav\n\
         STACKED = 7 STACK=duck\n\
         5 = 6\n\
         MISSING = custom/none.wav\n",
    );
    for f in [
        "boom1.wav",
        "boom2.wav",
        "step.wav",
        "override.wav",
        "first.wav",
        "second.wav",
    ] {
        l.wav(
            &format!("custom/{f}"),
            match f {
                "boom1.wav" => 31,
                "boom2.wav" => 32,
                "step.wav" => 40,
                "override.wav" => 50,
                "first.wav" => 61,
                _ => 62,
            },
        );
    }
    let ctx = Context::new(&l.root, "eng");
    let a = Audio::with_banks(ctx, Some(mini_bank(BankKind::Effects, 40)), None);
    // raw id redirect: 36 and 37 -> boom1.wav, boom2.wav (count 2: with no digit in the stem the
    // game numbers from 1, so `boom.wav` itself is NOT a candidate, contrary to the comment in
    // fxdata/sounds.cfg), also reachable by the alias
    for (id, frames) in [("36", 31), ("37", 32)] {
        let r = a.resolve(id).unwrap();
        assert_eq!(r.layer.kind, LayerKind::Install);
        assert!(r.note.as_deref().unwrap().starts_with("redirect"));
        assert_eq!(frames_of(&a, &r.variants[0].source), frames);
    }
    let r = a.resolve("my_boom").unwrap();
    assert_eq!(r.variants.len(), 2);
    assert_eq!(frames_of(&a, &r.variants[1].source), 32);
    assert_eq!(
        frames_of(&a, &a.resolve("9").unwrap().variants[0].source),
        40
    );
    // the redirected id is also what a numeric name pointing at it plays
    assert_eq!(
        a.resolve("38").unwrap().variants[0].source,
        Source::Bank {
            kind: BankKind::Effects,
            index: 38
        }
    );
    // a custom file named NAME beats NAME = 5 whatever the order
    let r = a.resolve("NAME").unwrap();
    assert_eq!(frames_of(&a, &r.variants[0].source), 50);
    // the first custom definition of a name keeps its file (the game's loadCustomSound returns the
    // loaded entry)
    assert_eq!(
        frames_of(&a, &a.resolve("DUP").unwrap().variants[0].source),
        61
    );
    // "STACKED = 7 STACK=duck": the second token is taken as the count (atoi = 0 -> 1), STACK is
    // not read
    let r = a.resolve("STACKED").unwrap();
    assert_eq!((r.variants.len(), r.declared_count), (1, 1));
    // a numeric value for a numeric key is refused; a missing file is a warning, not a name
    assert!(a.resolve("5").is_ok());
    assert!(a.resolve("MISSING").is_err());
    assert!(
        a.warnings.iter().any(|w| w.contains("MISSING")),
        "{:?}",
        a.warnings
    );
}

#[test]
fn file_search_order_and_extensions() {
    let l = Layout::new("search");
    l.write(
        "fxdata/sounds.cfg",
        "[sounds]\nA = snd/a\nB = snd/b\nC = snd/c.ogg\nD = snd/d\nZ = \
        zipped\nZ2 = sound/zipped2.wav\n",
    );
    l.write(
        "campgns/test.cfg",
        "[common]\nLEVELS_LOCATION = levels/test\nCONFIGS_LOCATION = \
        levels/test_cfgs\nMEDIA_LOCATION = ldata\nCREATURES_LOCATION = levels/test_crtr\n",
    );
    l.write("levels/test_cfgs/sounds.cfg", "[sounds]\n");
    // a: present in media, install root and the campaign configs folder; extension candidates .wav
    // first
    l.wav("ldata/snd/a.wav", 1);
    l.wav("snd/a.wav", 2);
    l.wav("levels/test_cfgs/snd/a.wav", 3);
    // b: .mp3 in the levels folder (listed before configs) but a .wav in the root: the extension
    // loop is the OUTER loop
    l.write("levels/test/snd/b.mp3", "not really mp3");
    l.wav("snd/b.wav", 5);
    // mods: sound/ dirs only
    l.write(
        "mods/load_order.cfg",
        "[after_base]\nm1\nm2\n[after_campaign]\n[after_map]\n",
    );
    l.wav("mods/m1/sound/snd/c.wav", 7);
    l.wav("mods/m2/sound/snd/a.wav", 9);
    l.wav("mods/m1/sound/snd/d.wav", 10);
    l.wav("mods/m2/sound/snd/d.wav", 11);
    l.wav("mods/m1/fxdata/snd/a.wav", 99); // not a sound folder: ignored
    l.zip(
        "levels/test/map00007.zip",
        &[
            (
                "sound/zipped.wav",
                Pcm::new(8000, 1, vec![0.2; 77]).to_wav_bytes(),
            ),
            (
                "Sound/Zipped2.WAV",
                Pcm::new(8000, 1, vec![0.2; 78]).to_wav_bytes(),
            ),
        ],
    );
    let ctx = Context::new(&l.root, "eng")
        .with_campaign(&l.root.join("campgns/test.cfg"))
        .unwrap()
        .with_level(7);
    let a = Audio::with_banks(ctx, None, None);
    let get = |n: &str| a.resolve(n).map(|r| r.variants[0].clone());
    // campaign configs folder beats the media folder, the install root and the after_base mods; m2
    // over m1 (search order: after_map mods, levels, after_campaign mods, configs, after_base mods,
    // fxdata, media, root)
    assert_eq!(frames_of(&a, &get("A").unwrap().source), 3);
    // the .wav extension candidate is tried in every folder before .mp3: snd/b.wav in the root is
    // found
    let b = get("B").unwrap();
    assert_eq!(frames_of(&a, &b.source), 5, "{b:?}");
    // explicit extension: only that file is looked for (snd/c.wav in m1 is not a candidate for
    // c.ogg)
    assert!(get("C").is_err());
    assert!(
        a.warnings.iter().any(|w| w.contains("sound C")),
        "{:?}",
        a.warnings
    );
    // mods: the later mod is searched first
    assert_eq!(frames_of(&a, &get("D").unwrap().source), 11);
    // the map zip is the last resort; "sound/" is stripped from the cfg path and entries match
    // case-insensitively
    let z = get("Z").unwrap();
    assert!(
        matches!(&z.source, Source::ZipEntry { entry, .. } if entry == "sound/zipped.wav"),
        "{z:?}"
    );
    assert_eq!(frames_of(&a, &z.source), 77);
    let z2 = get("Z2").unwrap();
    assert_eq!(frames_of(&a, &z2.source), 78);
    // without a selected level there is no zip
    let ctx = Context::new(&l.root, "eng")
        .with_campaign(&l.root.join("campgns/test.cfg"))
        .unwrap();
    let a2 = Audio::with_banks(ctx, None, None);
    assert!(a2.resolve("Z").is_err());
}

#[test]
fn numbered_expansion_like_the_game() {
    assert_eq!(
        expand_numbered("creature/maiden/hit1.wav", 4),
        [
            "creature/maiden/hit1.wav",
            "creature/maiden/hit2.wav",
            "creature/maiden/hit3.wav",
            "creature/maiden/hit4.wav"
        ]
    );
    assert_eq!(
        expand_numbered("SHE_ORC_Hit_08.mp3", 3),
        [
            "SHE_ORC_Hit_08.mp3",
            "SHE_ORC_Hit_09.mp3",
            "SHE_ORC_Hit_10.mp3"
        ]
    );
    assert_eq!(expand_numbered("boom", 2), ["boom1", "boom2"]);
    assert_eq!(
        expand_numbered("a.wav", 1),
        ["a.wav"],
        "count 1 keeps the path"
    );
    assert_eq!(expand_numbered("a.wav", 2), ["a1.wav", "a2.wav"]);
    assert_eq!(
        kfx_audio::context::expand_creature_numbered("creature/imp/hit1.wav", 3),
        [
            "creature/imp/hit1.wav",
            "creature/imp/hit2.wav",
            "creature/imp/hit3.wav"
        ]
    );
    assert_eq!(
        kfx_audio::context::expand_creature_numbered("hit.wav", 2),
        ["hit.wav", "hit.wav"],
        "no digits: copies of the same path"
    );
    assert_eq!(
        kfx_audio::context::expand_creature_numbered("d2/hit.wav", 2),
        ["d2/hit.wav", "d2/hit.wav"],
        "digits of the directory do not count"
    );
    assert_eq!(expand_numbered("x9.wav", 40).len(), 32, "capped at 32");
    assert_eq!(
        expand_numbered("dir.v2/clip.wav", 2),
        ["dir.v2/clip1.wav", "dir.v2/clip2.wav"]
    );
}

#[test]
fn speech_overrides_creatures_level_speech_and_music() {
    let l = Layout::new("misc");
    l.write(
        "fxdata/sounds.cfg",
        "[sounds]\nX = 1\n[speech]\nGoldLow = speech/gold.wav\nHello = \
        none\nPayday = speech/payday.wav\nGlaagh = speech/glaagh.wav\n",
    );
    l.write(
        "campgns/test.cfg",
        "[common]\nNAME = Test\nLEVELS_LOCATION = levels/test\nCONFIGS_LOCATION = \
         levels/test_cfgs\nCREATURES_LOCATION = levels/test_crtr\nMEDIA_LOCATION = ldata\n\
         SOUNDTRACK = 3\n[speech]\nENG = campgns/test_eng\nGER = campgns/test_ger\n\
         [map00001]\nSPEECH = good01 bad01.mp3\n",
    );
    l.write(
        "levels/test_cfgs/sounds.cfg",
        "[speech]\nHello = speech/hello.wav\n",
    );
    // speech: language folder first, then English, then the base path
    l.wav("ldata/speech/gold.wav", 10);
    l.wav("ldata/speech/ger/gold.wav", 11);
    l.wav("ldata/speech/eng/payday.wav", 12);
    l.wav("ldata/speech/payday.wav", 13);
    l.wav("ldata/speech/hello.wav", 14);
    l.wav("ldata/speech/fre/Glaagh.wav", 15);
    l.wav("campgns/test_eng/good01.wav", 16);
    l.write("campgns/test_eng/bad01.mp3", "x");
    let ctx_for = |lang: &str| {
        Context::new(&l.root, lang)
            .with_campaign(&l.root.join("campgns/test.cfg"))
            .unwrap()
    };
    let eng = Audio::with_banks(ctx_for("eng"), None, Some(mini_bank(BankKind::Speech, 125)));
    let ger = Audio::with_banks(ctx_for("ger"), None, Some(mini_bank(BankKind::Speech, 125)));
    let frames = |a: &Audio, q: &str| {
        let r = a.resolve_speech(q).unwrap();
        a.decode(&r.variants[0].source).unwrap().frames()
    };
    assert_eq!(
        frames(&eng, "GoldLow"),
        10,
        "english install: base path (no eng/gold.wav)"
    );
    assert_eq!(frames(&ger, "goldlow"), 11, "language folder wins");
    assert_eq!(
        frames(&ger, "Payday"),
        12,
        "German install without German file falls to English before the base path"
    );
    assert_eq!(frames(&eng, "Payday"), 12);
    // [speech] from the campaign layer overrides the base one for the same message
    assert_eq!(frames(&eng, "Hello"), 14);
    assert_eq!(
        eng.resolve_speech("Hello").unwrap().layer.kind,
        LayerKind::Campaign
    );
    // a speech file that exists only in some other language folder is still found (step 4)
    let r = eng.resolve_speech("Glaagh").unwrap();
    assert!(
        r.note
            .as_deref()
            .unwrap()
            .contains("first available language folder"),
        "{r:?}"
    );
    // an override whose file does not exist plays nothing: no variants (no fallback to the bank)
    let l2 = Layout::new("misc2");
    l2.write(
        "fxdata/sounds.cfg",
        "[speech]\nGoldLow = speech/none_here.wav\nHello = null\n",
    );
    let a = Audio::with_banks(
        Context::new(&l2.root, "eng"),
        None,
        Some(mini_bank(BankKind::Speech, 125)),
    );
    assert!(a.resolve_speech("GoldLow").unwrap().variants.is_empty());
    assert_eq!(
        a.resolve_speech("Hello").unwrap().variants[0].source,
        Source::Silence
    );
    // no override: the bank message by name and by index (the SMsg enum value is the bank index)
    let b = Audio::with_banks(
        Context::new(&l2.root, "eng"),
        None,
        Some(mini_bank(BankKind::Speech, 125)),
    );
    assert_eq!(
        b.resolve_speech("Payday").unwrap().variants[0].source,
        Source::Bank {
            kind: BankKind::Speech,
            index: 90
        }
    );
    assert_eq!(
        b.resolve_speech("GameFinalVictory").unwrap().variants[0].source,
        Source::Bank {
            kind: BankKind::Speech,
            index: 109
        }
    );
    assert_eq!(
        b.resolve_speech("7").unwrap().variants[0].source,
        Source::Bank {
            kind: BankKind::Speech,
            index: 7
        }
    );
    assert!(b.resolve_speech("NoSuchMessage").is_err());

    // level speech: campaign [speech] folder, ".wav" appended without extension
    let r = eng.resolve_level_speech(1, true).unwrap();
    assert_eq!(file_name(&r.variants[0].source), "good01.wav");
    let r = eng.resolve_level_speech(1, false).unwrap();
    assert_eq!(file_name(&r.variants[0].source), "bad01.mp3");
    assert!(eng.resolve_level_speech(2, true).is_err());
    let r = ger.resolve_level_speech(1, true).unwrap();
    assert!(
        r.variants.is_empty(),
        "German speech folder campgns/test_ger does not exist: {r:?}"
    );

    // creature sets: install, campaign, level layers; bank ids, files with counts, NONE
    l.write(
        "creatrs/imp.cfg",
        "[attributes]\nHealth = 1\n[sounds]\nFoot = 9 4\nHit = 20\nHappy = \
        hit1.wav 2\nDie = NONE\n",
    );
    l.wav("creatrs/hit1.wav", 3);
    l.wav("creatrs/hit2.wav", 4);
    l.write(
        "levels/test_crtr/imp.cfg",
        "[sounds]\nHit = 21 2\nSad = custom/sad.wav\n",
    );
    l.wav("levels/test_crtr/custom/sad.wav", 5);
    l.write("levels/test/map00001.imp.cfg", "[sounds]\nFoot = 30\n");
    l.write("creatrs/orc.cfg", "[sounds]\nFoot = 1\n");
    let a = Audio::with_banks(
        ctx_for("eng").with_level(1),
        Some(mini_bank(BankKind::Effects, 60)),
        None,
    );
    let imp = a.creature_set("IMP");
    let slot = |s: kfx_audio::CreatureSound| {
        imp.slots
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, r)| r.clone())
    };
    use kfx_audio::CreatureSound as Cs;
    let foot = slot(Cs::Foot).unwrap();
    assert_eq!(foot.layer.kind, LayerKind::Level);
    assert_eq!(
        foot.variants[0].source,
        Source::Bank {
            kind: BankKind::Effects,
            index: 30
        }
    );
    let hit = slot(Cs::Hit).unwrap();
    assert_eq!(
        (hit.layer.kind, hit.variants.len(), hit.declared_count),
        (LayerKind::Campaign, 2, 2)
    );
    assert_eq!(
        hit.variants[1].source,
        Source::Bank {
            kind: BankKind::Effects,
            index: 22
        }
    );
    let happy = slot(Cs::Happy).unwrap();
    assert_eq!(
        happy
            .variants
            .iter()
            .map(|v| file_name(&v.source))
            .collect::<Vec<_>>(),
        ["hit1.wav", "hit2.wav"]
    );
    assert_eq!(slot(Cs::Die).unwrap().variants[0].source, Source::Silence);
    assert_eq!(frames_of(&a, &slot(Cs::Sad).unwrap().variants[0].source), 5);
    assert!(
        slot(Cs::Slap).is_none(),
        "slots the cfg never mentions are absent"
    );
    assert_eq!(a.creature_names(), ["imp", "orc"]);
    // without the level only the campaign / install layers apply
    let a = Audio::with_banks(ctx_for("eng"), Some(mini_bank(BankKind::Effects, 60)), None);
    let foot = a
        .creature_set("imp")
        .slots
        .iter()
        .find(|(k, _)| *k == Cs::Foot)
        .unwrap()
        .1
        .clone();
    assert_eq!(
        (foot.layer.kind, foot.declared_count),
        (LayerKind::Install, 4)
    );

    // music: numbered tracks start at 2 and use the best extension present
    l.write("music/b_track.mp3", "x");
    l.write("music/a_track.mp3", "x");
    l.write("music/Z.OGG", "x");
    l.write("music/MusicReadme.txt", "x");
    l.write("music/a.ogg", "x");
    let tracks = a.music_tracks();
    assert_eq!(
        tracks
            .iter()
            .map(|t| (
                t.number,
                t.path.file_name().unwrap().to_string_lossy().to_string()
            ))
            .collect::<Vec<_>>(),
        [(2, "a.ogg".to_string()), (3, "Z.OGG".to_string())]
    );
    assert!(a.resolve_music_track(1).is_err() && a.resolve_music_track(4).is_err());
    assert_eq!(
        file_name(&a.resolve_music_track(3).unwrap().variants[0].source),
        "Z.OGG"
    );
    // SOUNDTRACK = 3 is a track number
    assert_eq!(
        file_name(&a.resolve_soundtrack().unwrap().variants[0].source),
        "Z.OGG"
    );
    // custom music files: media folder first
    l.write("ldata/song.mp3", "x");
    assert_eq!(
        a.resolve_music_file("song.mp3").unwrap().layer.kind,
        LayerKind::Campaign
    );
    assert!(a.resolve_music_file("nothing.mp3").is_err());
}

#[test]
fn catalog_lists_all_categories() {
    let l = Layout::new("catalog");
    l.write("fxdata/sounds.cfg", "[sounds]\nTAB = 2\nFILE = f.wav\n");
    l.wav("f.wav", 5);
    l.write("creatrs/imp.cfg", "[sounds]\nFoot = 1 2\n");
    l.write("music/t.ogg", "x");
    let a = Audio::with_banks(
        Context::new(&l.root, "eng"),
        Some(mini_bank(BankKind::Effects, 4)),
        Some(mini_bank(BankKind::Speech, 3)),
    );
    let cat = a.catalog();
    let of = |c: kfx_audio::Category| cat.iter().filter(|e| e.category == c).count();
    assert_eq!(
        of(kfx_audio::Category::Effects),
        4 + 1,
        "4 bank samples + the FILE custom sound"
    );
    assert_eq!(of(kfx_audio::Category::Speech), 3);
    assert_eq!(of(kfx_audio::Category::Creatures), 1);
    assert_eq!(of(kfx_audio::Category::Music), 1);
    let e = cat.iter().find(|e| e.key == "fx:2").unwrap();
    assert!(e.detail.contains("TAB"), "{e:?}");
    assert!((e.duration.unwrap() - 200.0 / 22050.0).abs() < 1e-9);
    let keys: std::collections::HashSet<_> = cat.iter().map(|e| e.key.clone()).collect();
    assert_eq!(keys.len(), cat.len(), "keys are unique");
}
