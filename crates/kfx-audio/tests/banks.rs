//! Every sample of every sound bank of the test install decodes; counts by codec; golden numbers.
use kfx_audio::bank::{BankKind, SoundBank};
use kfx_audio::codec::Codec;
use kfx_formats::testutil::{install_dir, resolve_ci};
use std::collections::BTreeMap;

fn banks() -> Vec<(String, SoundBank)> {
    let Some(install) = install_dir() else {
        eprintln!("no KFX_INSTALL: skipped");
        return vec![];
    };
    let dir = resolve_ci(&install, "sound").unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".dat") && (n == "sound.dat" || n.starts_with("speech")))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|n| {
            let kind = if n == "sound.dat" {
                BankKind::Effects
            } else {
                BankKind::Speech
            };
            let b = SoundBank::load(kind, &dir.join(&n)).unwrap_or_else(|e| panic!("{n}: {e}"));
            (n, b)
        })
        .collect()
}

#[test]
fn every_sample_decodes() {
    let mut by_codec: BTreeMap<Codec, usize> = BTreeMap::new();
    let mut total = 0f64;
    let mut count = 0usize;
    let mut failures = vec![];
    let mut silent = vec![];
    let mut clipped_files = 0usize;
    for (name, b) in banks() {
        let mut bank_secs = 0.0;
        for s in b.samples.iter().filter(|s| !s.reserved) {
            *by_codec.entry(s.codec).or_default() += 1;
            match b.decode(s.index as usize) {
                Ok(p) => {
                    let st = p.stats();
                    count += 1;
                    bank_secs += st.duration_secs;
                    if st.silent {
                        silent.push(format!("{name}#{} {}", s.index, s.name));
                    }
                    if st.clipped > p.samples.len() / 100 {
                        clipped_files += 1;
                    }
                    // the header says how long it is
                    let hdr = s.duration_secs();
                    assert!(
                        (hdr - st.duration_secs).abs() < 0.01,
                        "{name}#{} header {hdr} decoded {}",
                        s.index,
                        st.duration_secs
                    );
                    assert!(
                        p.sample_rate >= 8000 && p.sample_rate <= 96000,
                        "{name}#{} rate {}",
                        s.index,
                        p.sample_rate
                    );
                }
                Err(e) => failures.push(format!("{name}#{} {}: {e}", s.index, s.name)),
            }
        }
        eprintln!(
            "{name}: {} samples (incl. reserved 0), {:.1} s",
            b.len(),
            bank_secs
        );
        total += bank_secs;
    }
    eprintln!("decoded {count} samples, {total:.1} s total");
    eprintln!("by codec: {by_codec:?}");
    eprintln!("silent: {silent:?}");
    eprintln!("files with more than 1% clipped samples: {clipped_files}");
    assert!(failures.is_empty(), "failures: {failures:?}");
}

#[test]
fn counts_and_codecs() {
    let all = banks();
    if all.is_empty() {
        return;
    }
    let mut by_codec: BTreeMap<Codec, usize> = BTreeMap::new();
    for (name, b) in &all {
        let playable = b.samples.iter().filter(|s| !s.reserved).count();
        if name == "sound.dat" {
            assert_eq!(playable, 1067);
        } else {
            assert_eq!(playable, 125, "{name}");
        }
        assert!(b.samples[0].reserved && b.samples[0].name.is_empty());
        assert!(
            b.samples.iter().all(|s| !s.broken),
            "{name}: broken headers"
        );
        for s in b.samples.iter().filter(|s| !s.reserved) {
            *by_codec.entry(s.codec).or_default() += 1;
        }
    }
    // what occurs in the install: only PCM 8/16 and exactly one MS ADPCM clip (heart6a.wav)
    assert_eq!(by_codec.get(&Codec::MsAdpcm), Some(&1));
    assert_eq!(by_codec.get(&Codec::Pcm8), Some(&1423));
    assert_eq!(by_codec.get(&Codec::Pcm16), Some(&1393));
    assert_eq!(by_codec.len(), 3, "{by_codec:?}");
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 2e-4
}

#[test]
fn golden_samples() {
    let all = banks();
    let get = |n: &str| all.iter().find(|(f, _)| f == n);
    let Some((_, fx)) = get("sound.dat") else {
        return;
    };
    let (_, sp) = get("speech_eng.dat").unwrap();

    let k = fx.find("foot1a.wav").unwrap();
    let s = &fx.samples[k];
    assert_eq!((k, s.offset, s.size, s.sfx_id), (1, 0, 31344, 0));
    let p = fx.decode(k).unwrap();
    let st = p.stats();
    assert_eq!(
        (s.codec, p.channels, p.sample_rate, p.frames()),
        (Codec::Pcm16, 1, 22050, 15649)
    );
    assert!(near(st.peak, 0.64932) && near(st.rms, 0.03696), "{st:?}");
    assert_eq!(st.clipped, 0);

    let k = fx.find("heart6a.wav").unwrap();
    let p = fx.decode(k).unwrap();
    let st = p.stats();
    assert_eq!(
        (fx.samples[k].codec, p.channels, p.sample_rate, p.frames()),
        (Codec::MsAdpcm, 1, 22050, 43640)
    );
    // an independent decoder (ffmpeg) agrees to one LSB; the game's own ADPCM path does not decode
    // it properly
    assert!(near(st.peak, 0.52591) && near(st.rms, 0.13224), "{st:?}");

    let k = fx.find("maidfgt3.wav").unwrap();
    let p = fx.decode(k).unwrap();
    assert_eq!(
        (fx.samples[k].codec, p.sample_rate, p.frames()),
        (Codec::Pcm8, 48000, 73756)
    );

    let k = sp.find("igame_angry_gener").unwrap();
    let p = sp.decode(k).unwrap();
    let st = p.stats();
    assert_eq!(
        (k, sp.samples[k].codec, p.sample_rate, p.frames()),
        (1, Codec::Pcm8, 22050, 42239)
    );
    assert!(near(st.peak, 0.99219) && near(st.rms, 0.22466), "{st:?}");
}

/// Decode, write a WAV file, read it back: duration, sample rate, rms and clipping of the file.
#[test]
fn wav_round_trip_inspection() {
    let all = banks();
    let Some((_, fx)) = all.iter().find(|(f, _)| f == "sound.dat") else {
        return;
    };
    let dir = kfx_formats::testutil::scratch_dir("kfx-audio/test-roundtrip");
    for n in ["foot1a.wav", "heart6a.wav"] {
        let k = fx.find(n).unwrap();
        let p = fx.decode(k).unwrap();
        let path = dir.join(n);
        std::fs::write(&path, p.to_wav_bytes()).unwrap();
        let (back, codec) = kfx_audio::codec::decode_wav(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(codec, Codec::Pcm16);
        assert_eq!(
            (back.sample_rate, back.channels, back.frames()),
            (p.sample_rate, p.channels, p.frames())
        );
        let st = back.stats();
        assert!(
            !st.silent && st.clipped == 0 && st.rms > 0.01,
            "{n}: {st:?}"
        );
        assert!((st.duration_secs - p.duration_secs()).abs() < 1e-9);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
