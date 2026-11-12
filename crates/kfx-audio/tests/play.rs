//! The null backend and the mixer. No audio device is ever opened here.
use kfx_audio::play::{AudioBackend, Mixer, NullBackend, PlayOptions, Player};
use kfx_audio::Pcm;
use std::sync::Arc;

fn tone(rate: u32, secs: f64, amp: f32) -> Arc<Pcm> {
    let n = (rate as f64 * secs) as usize;
    Arc::new(Pcm::new(
        rate,
        1,
        (0..n).map(|i| amp * (i as f32 * 0.1).sin()).collect(),
    ))
}

#[test]
fn voices_end_and_report_position() {
    let mut b = NullBackend::new();
    b.realtime = false;
    let id = b
        .play(tone(22050, 1.0, 0.5), PlayOptions::default())
        .unwrap();
    assert!(b.is_playing(id));
    assert_eq!(b.active_voices(), 1);
    b.advance(0.4);
    let pos = b.position_secs(id).unwrap();
    assert!((pos - 0.4).abs() < 0.01, "{pos}");
    assert!(b.is_playing(id));
    b.advance(0.7);
    assert!(!b.is_playing(id), "ended after its 1 s");
    assert_eq!(b.active_voices(), 0);
    assert!(b.position_secs(id).is_none());
}

#[test]
fn stop_stop_all_and_empty_clips() {
    let mut b = NullBackend::new();
    let a = b
        .play(tone(8000, 2.0, 0.5), PlayOptions::default())
        .unwrap();
    let c = b
        .play(tone(8000, 2.0, 0.5), PlayOptions::default())
        .unwrap();
    b.stop(a);
    assert!(!b.is_playing(a) && b.is_playing(c));
    b.stop_all();
    assert_eq!(b.active_voices(), 0);
    // an empty clip never occupies a voice
    let e = b
        .play(Arc::new(Pcm::new(8000, 1, vec![])), PlayOptions::default())
        .unwrap();
    assert!(!b.is_playing(e));
}

#[test]
fn voice_limit_steals_the_oldest() {
    let mut b = NullBackend::new();
    b.set_max_voices(3);
    let ids: Vec<_> = (0..5)
        .map(|_| {
            b.play(tone(8000, 5.0, 0.1), PlayOptions::default())
                .unwrap()
        })
        .collect();
    assert_eq!(b.active_voices(), 3);
    assert!(!b.is_playing(ids[0]) && !b.is_playing(ids[1]));
    assert!(ids[2..].iter().all(|&i| b.is_playing(i)));
}

#[test]
fn volume_pan_loop_and_meter() {
    let mut b = NullBackend::new();
    b.realtime = false;
    b.play(
        tone(44100, 0.5, 0.5),
        PlayOptions {
            volume: 1.0,
            ..Default::default()
        },
    )
    .unwrap();
    b.advance(0.1);
    let loud = b.meter();
    assert!(
        loud.peak[0] > 0.3 && loud.peak[1] > 0.3 && loud.rms > 0.1,
        "{loud:?}"
    );
    assert_eq!(loud.scope.len(), 512);
    b.stop_all();
    b.advance(1.0);
    let quiet = b.meter();
    assert!(quiet.peak[0] < 0.01 && quiet.rms < 1e-6, "{quiet:?}");
    // half volume is half the peak
    b.play(
        tone(44100, 0.5, 0.5),
        PlayOptions {
            volume: 0.5,
            ..Default::default()
        },
    )
    .unwrap();
    b.advance(0.1);
    let half = b.meter();
    assert!(half.peak[0] < loud.peak[0] * 0.7, "{half:?} vs {loud:?}");
    // hard left pan: right channel is quiet (at -1.0 the right gain is 0.5 of centre in this
    // equal-ish law)
    b.stop_all();
    b.advance(1.0);
    b.play(
        tone(44100, 0.5, 0.5),
        PlayOptions {
            pan: -1.0,
            ..Default::default()
        },
    )
    .unwrap();
    b.advance(0.1);
    let left = b.meter();
    assert!(left.peak[0] > left.peak[1] * 1.5, "{left:?}");
    // looping voices never end
    b.stop_all();
    let l = b
        .play(
            tone(8000, 0.1, 0.5),
            PlayOptions {
                looping: true,
                ..Default::default()
            },
        )
        .unwrap();
    b.advance(1.0);
    assert!(b.is_playing(l));
    // master volume 0 mutes
    b.set_master_volume(0.0);
    b.advance(0.1);
    assert!(b.meter().rms < 1e-6);
}

#[test]
fn mixer_resamples_and_clips() {
    let mut m = Mixer::default();
    // 1 second at 1000 Hz rendered at 4000 Hz lasts 4000 frames
    m.play(
        Arc::new(Pcm::new(1000, 1, vec![0.5; 1000])),
        PlayOptions::default(),
    );
    let mut out = vec![0f32; 2 * 4000];
    m.render(&mut out, 4000);
    assert!(
        out.chunks(2)
            .all(|f| (f[0] - 0.5).abs() < 1e-3 || f[0] == 0.0),
        "centre pan keeps full gain"
    );
    assert_eq!(m.active(), 0);
    // two loud voices clip at +-1
    let loud = Arc::new(Pcm::new(8000, 1, vec![0.9; 100]));
    m.play(loud.clone(), PlayOptions::default());
    m.play(loud, PlayOptions::default());
    let mut out = vec![0f32; 2 * 50];
    m.render(&mut out, 8000);
    assert!(out.iter().all(|&s| s <= 1.0) && out.iter().any(|&s| s > 0.99));
}

#[test]
fn player_caches_and_replaces_the_preview() {
    let mut p = Player::null();
    let mut decodes = 0;
    let a = p
        .clip("k", || {
            decodes += 1;
            Ok((*tone(8000, 1.0, 0.3)).clone())
        })
        .unwrap();
    let _ = p
        .clip("k", || {
            decodes += 1;
            Ok((*tone(8000, 1.0, 0.3)).clone())
        })
        .unwrap();
    assert_eq!(decodes, 1);
    let v1 = p.play_clip(a.clone(), PlayOptions::default()).unwrap();
    assert!(p.is_playing());
    let v2 = p.play_clip(a, PlayOptions::default()).unwrap();
    assert!(
        !p.backend.is_playing(v1) && p.backend.is_playing(v2),
        "a new preview stops the old one"
    );
    p.stop();
    assert!(!p.is_playing());
}

/// Real output needs a device and makes noise: only with `KFX_AUDIO_PLAYBACK_TEST=1` (off by
/// default), and even then only opens the stream and plays a clip at volume 0.
#[cfg(feature = "playback")]
#[test]
fn cpal_backend_opens_silently_when_asked() {
    if std::env::var_os("KFX_AUDIO_PLAYBACK_TEST").is_none() {
        eprintln!("KFX_AUDIO_PLAYBACK_TEST not set: skipped");
        return;
    }
    let mut b = kfx_audio::play::CpalBackend::new().expect("output device");
    b.set_master_volume(0.0);
    let id = b
        .play(tone(22050, 0.2, 0.5), PlayOptions::default())
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(!b.is_playing(id));
}
