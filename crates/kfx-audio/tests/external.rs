//! Custom sound formats: MP3 from the install, `BMU V1.0`, OGG/FLAC made with ffmpeg when it is
//! installed (never played, only decoded and measured), synthetic WAVs, malformed input.
use kfx_audio::codec::{decode_audio, decode_wav, sniff, Codec};
use kfx_formats::testutil::{install_dir, resolve_ci};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    kfx_formats::testutil::scratch_dir(&format!("kfx-audio/{name}"))
}

fn wav_header(tag: u16, ch: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    v.extend((36 + data.len() as u32).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(tag.to_le_bytes());
    v.extend(ch.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * ch as u32 * (bits as u32 / 8)).to_le_bytes());
    v.extend((ch * (bits / 8)).to_le_bytes());
    v.extend(bits.to_le_bytes());
    v.extend(b"data");
    v.extend((data.len() as u32).to_le_bytes());
    v.extend(data);
    v
}

#[test]
fn synthetic_wav_formats() {
    // 8-bit unsigned: 128 is silence, 255 is almost +1
    let (p, c) = decode_wav(&wav_header(1, 1, 8000, 8, &[128, 255, 0, 128])).unwrap();
    assert_eq!(c, Codec::Pcm8);
    assert_eq!(p.samples[0], 0.0);
    assert!((p.samples[1] - 127.0 / 128.0).abs() < 1e-6 && p.samples[2] == -1.0);
    // 16-bit stereo
    let d: Vec<u8> = [0i16, 16384, -32768, 32767]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let (p, c) = decode_wav(&wav_header(1, 2, 44100, 16, &d)).unwrap();
    assert_eq!((c, p.channels, p.frames()), (Codec::Pcm16, 2, 2));
    assert_eq!(p.samples, vec![0.0, 0.5, -1.0, 32767.0 / 32768.0]);
    // 24-bit
    let (p, c) = decode_wav(&wav_header(
        1,
        1,
        8000,
        24,
        &[0x00, 0x00, 0x40, 0x00, 0x00, 0xC0],
    ))
    .unwrap();
    assert_eq!(c, Codec::Pcm24);
    assert_eq!(p.samples, vec![0.5, -0.5]);
    // float
    let d: Vec<u8> = [0.25f32, -1.0]
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    let (p, c) = decode_wav(&wav_header(3, 1, 8000, 32, &d)).unwrap();
    assert_eq!((c, p.samples), (Codec::Float, vec![0.25, -1.0]));
    // a data chunk larger than the file is clamped; an odd trailing byte is dropped
    let mut w = wav_header(1, 1, 8000, 16, &[1, 0, 2, 0, 3]);
    w[40..44].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(decode_wav(&w).unwrap().0.frames(), 2);
    // unsupported tag is reported, not decoded
    assert!(matches!(
        decode_wav(&wav_header(0x55, 1, 8000, 16, &[0; 4])),
        Err(kfx_audio::Error::Unsupported(_))
    ));
}

#[test]
fn malformed_input_never_panics() {
    let mut w = wav_header(1, 2, 22050, 16, &[1; 64]);
    for cut in 0..w.len() {
        let _ = decode_wav(&w[..cut]);
        let _ = decode_audio(&w[..cut], "x.wav");
    }
    // corrupt every byte of the header region
    for i in 0..48 {
        let keep = w[i];
        for v in [0u8, 1, 0x7f, 0xff] {
            w[i] = v;
            let _ = decode_wav(&w);
        }
        w[i] = keep;
    }
    for junk in [
        &b""[..],
        b"RIFF",
        b"OggS",
        b"ID3",
        b"\xff\xfb",
        b"fLaC",
        b"BMU V1.0",
        b"\0\0\0\0\0\0\0\0\0\0\0\0",
    ] {
        assert!(decode_audio(junk, "j.bin").is_err(), "{junk:?}");
    }
}

#[test]
fn mp3_from_the_install_and_bmu_wrapper() {
    let Some(install) = install_dir() else { return };
    let Some(mp3) = resolve_ci(&install, "campgns/keeporig_eng/good01.mp3").filter(|p| p.is_file())
    else {
        return;
    };
    let bytes = std::fs::read(&mp3).unwrap();
    assert_eq!(sniff(&bytes), Codec::Mp3);
    let (p, c) = decode_audio(&bytes, "good01.mp3").unwrap();
    let st = p.stats();
    eprintln!(
        "good01.mp3: {:?} {} Hz {} ch {:.2} s peak {:.3} rms {:.4} clipped {}",
        c, p.sample_rate, p.channels, st.duration_secs, st.peak, st.rms, st.clipped
    );
    assert_eq!(c, Codec::Mp3);
    assert!(matches!(
        p.sample_rate,
        22050 | 24000 | 32000 | 44100 | 48000
    ));
    assert!(st.duration_secs > 5.0 && st.duration_secs < 600.0, "{st:?}");
    assert!(!st.silent && st.rms > 0.005 && st.peak <= 1.2, "{st:?}");
    // the wrapper some campaigns use: 8 byte prefix in front of a plain MP3
    // (`decode_audio_buffer_and_store`)
    let mut wrapped = b"BMU V1.0".to_vec();
    wrapped.extend(&bytes);
    let (q, c) = decode_audio(&wrapped, "x.bmu").unwrap();
    assert_eq!(c, Codec::Mp3);
    assert_eq!(q.samples.len(), p.samples.len());
}

#[test]
fn every_mp3_of_the_install_decodes() {
    let Some(install) = install_dir() else { return };
    let mut n = 0;
    let mut secs = 0.0;
    let mut stack = vec![install.join("campgns")];
    let mut failures = vec![];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp3")) {
                match decode_audio(&std::fs::read(&p).unwrap(), &p.to_string_lossy()) {
                    Ok((pcm, _)) => {
                        n += 1;
                        secs += pcm.duration_secs();
                        if pcm.stats().silent {
                            failures.push(format!("{} silent", p.display()));
                        }
                    }
                    Err(e) => failures.push(format!("{}: {e}", p.display())),
                }
            }
        }
    }
    eprintln!("{n} mp3 files decoded, {secs:.0} s");
    assert!(failures.is_empty(), "{failures:?}");
}

fn have_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// OGG Vorbis and FLAC: made with ffmpeg (a 0.5 s 440 Hz sine at amplitude 0.5; lavfi sine is 1/8
/// scale, hence volume=4), decoded, measured.
#[test]
fn ogg_and_flac_via_ffmpeg() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed: skipped");
        return;
    }
    let dir = scratch("test-external");
    for (file, codec, args) in [
        ("sine.ogg", Codec::Vorbis, vec!["-c:a", "libvorbis"]),
        ("sine.flac", Codec::Flac, vec!["-c:a", "flac"]),
        ("sine.wav", Codec::Pcm16, vec!["-c:a", "pcm_s16le"]),
    ] {
        let path = dir.join(file);
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.5:sample_rate=44100",
            "-af",
            "volume=4",
            "-ac",
            "1",
        ])
        .args(&args)
        .arg(&path);
        if !cmd.status().map(|s| s.success()).unwrap_or(false) {
            eprintln!("ffmpeg cannot write {file}: skipped");
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let (p, c) = decode_audio(&bytes, file).unwrap();
        let st = p.stats();
        eprintln!(
            "{file}: {:?} {} Hz {} ch {:.3} s peak {:.3} rms {:.4}",
            c, p.sample_rate, p.channels, st.duration_secs, st.peak, st.rms
        );
        assert_eq!(c, codec);
        assert_eq!((p.sample_rate, p.channels), (44100, 1));
        assert!((st.duration_secs - 0.5).abs() < 0.05, "{st:?}");
        // a 0.5 amplitude sine: peak ~0.5, rms ~0.354
        assert!(
            (st.peak - 0.5).abs() < 0.05 && (st.rms - 0.3536).abs() < 0.03,
            "{st:?}"
        );
        assert_eq!(st.clipped, 0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
