//! Every analysis over every shipped map (robustness, timing), sanity checks on known maps, the
//! list of shipped multiplayer maps by symmetry, and overview images under
//! `target/tmp/kfx-analysis/` (skipped without the test install).
mod common;

use kfx_analysis::analysis::Analysis;
use kfx_analysis::layer::{reach_layer, render_layer, render_overview, territory_layer};
use kfx_analysis::reach::Tier;
use kfx_analysis::rules::Rules;
use kfx_analysis::terrain::KindTable;
use std::time::Instant;

#[test]
fn all_shipped_maps() {
    if kfx_formats::testutil::install_dir().is_none() {
        return;
    }
    let mut times: Vec<(u64, String, (u16, u16))> = vec![];
    let mut multi: Vec<(String, Vec<String>, f32, String)> = vec![];
    let mut keeporig1 = None;
    let mut surprises = vec![];
    let mut largest: Option<common::Shipped> = None;
    let n = common::for_each_shipped(|m| {
        let name = format!(
            "{}/map{:05}",
            m.folder.file_name().unwrap().to_string_lossy(),
            m.level
        );
        let rules = Rules::from_game(&m.gd);
        let table = KindTable::new(&m.gd, &rules);
        let t0 = Instant::now();
        let a = Analysis::of_doc_with(&m.doc, &m.gd, rules, table, Default::default());
        let us = t0.elapsed().as_micros() as u64;
        times.push((us, name.clone(), (m.doc.dims.w, m.doc.dims.h)));
        let keepers = a.sites.keepers();
        // the shipped multiplayer maps are the packs under `multiplayer/`
        let is_mp = m
            .folder
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("multiplayer"));
        if keepers.len() >= 2 && is_mp {
            let modes: Vec<String> = a
                .symmetry
                .iter()
                .filter(|c| c.terrain_exact())
                .map(|c| {
                    format!(
                        "{:?}{}",
                        c.mode,
                        if c.thing_breaks.is_empty() {
                            ""
                        } else {
                            "(terrain)"
                        }
                    )
                })
                .collect();
            let best = a
                .symmetry
                .iter()
                .map(|c| c.match_fraction)
                .fold(0.0f32, f32::max);
            if modes.is_empty() {
                if let Some(c) = a
                    .symmetry
                    .iter()
                    .filter(|c| c.match_fraction >= 0.99)
                    .max_by(|a, b| a.match_fraction.total_cmp(&b.match_fraction))
                {
                    println!(
                        "  NEAR {name}: {:?} broken by {} slabs, e.g. {:?}",
                        c.mode,
                        c.slab_breaks.len(),
                        &c.slab_breaks[..c.slab_breaks.len().min(6)]
                    );
                }
            }
            let top = a
                .fairness
                .as_ref()
                .map(|f| format!("{:.3} {}", f.score, f.top(2).join("; ")))
                .unwrap_or_default();
            multi.push((name.clone(), modes, best, top));
        }
        for f in &a.fields {
            if a.economy
                .players
                .iter()
                .find(|p| p.player == f.player)
                .is_some_and(|p| p.portals.iter().all(|d| d.turns == kfx_analysis::nav::INF))
                && !a.sites.portals.is_empty()
            {
                surprises.push(format!(
                    "{name}: {} cannot reach any of the {} portals",
                    kfx_analysis::sites::owner_name(f.player),
                    a.sites.portals.len()
                ));
            }
        }
        if !a.pockets.is_empty() {
            let with_things: usize = a.pockets.iter().filter(|p| !p.things.is_empty()).count();
            if with_things > 0 {
                surprises.push(format!(
                    "{name}: {} isolated pockets, {with_things} with things",
                    a.pockets.len()
                ));
            }
        }
        if m.folder
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with("keeporig")
            && m.level == 1
        {
            let f = a.keeper(0).expect("keeporig 1 has a red heart");
            let p = &a.economy.players[0];
            let d = p.portals.first().expect("a portal").turns;
            println!(
                "keeporig map00001: portal {} turns ({:.0}s) away by digging, tier {:?}; gold \
                slabs {} ({} gold); gems {}",
                d,
                a.rules.seconds(d),
                f.tier(
                    a.sites.portals[p.portals[0].portal].centre.0,
                    a.sites.portals[p.portals[0].portal].centre.1
                ),
                p.gold_slabs,
                p.gold_value,
                p.gems
            );
            keeporig1 = Some((
                d,
                f.tier(a.sites.portals[0].centre.0, a.sites.portals[0].centre.1),
            ));
            let img = render_overview(&a, 6);
            common::write_png(
                &common::out_dir().join("shipped_keeporig1_overview.png"),
                &img,
            );
            if let Some(l) = reach_layer(&a, 0) {
                common::write_png(
                    &common::out_dir().join("shipped_keeporig1_reach.png"),
                    &render_layer(&a.terrain, &l, 6, 150),
                );
            }
        }
        if largest.as_ref().is_none_or(|l| {
            (l.doc.dims.w as u32 * l.doc.dims.h as u32)
                < (m.doc.dims.w as u32 * m.doc.dims.h as u32)
        }) {
            largest = Some(m);
        }
    });
    println!("analysed {n} shipped maps");
    times.sort();
    let total: u64 = times.iter().map(|t| t.0).sum();
    println!(
        "time per map: mean {:.1} ms, median {:.1} ms, max {:.1} ms ({} {:?})",
        total as f64 / n.max(1) as f64 / 1000.0,
        times[times.len() / 2].0 as f64 / 1000.0,
        times.last().unwrap().0 as f64 / 1000.0,
        times.last().unwrap().1,
        times.last().unwrap().2
    );
    for t in times.iter().rev().take(5) {
        println!("  slow: {:.1} ms {} {:?}", t.0 as f64 / 1000.0, t.1, t.2);
    }
    let (sym, asym): (Vec<_>, Vec<_>) = multi.iter().partition(|m| !m.1.is_empty());
    println!(
        "multiplayer maps (multiplayer/ packs, 2+ keepers): {} symmetric terrain, {} not",
        sym.len(),
        asym.len()
    );
    for m in &sym {
        println!("  SYMMETRIC {} {:?} fairness {}", m.0, m.1, m.3);
    }
    for m in &asym {
        println!(
            "  not symmetric {} (best mode matches {:.1}% of slabs) fairness {}",
            m.0,
            m.2 * 100.0,
            m.3
        );
    }
    for s in surprises.iter().take(40) {
        println!("  note: {s}");
    }
    assert!(n >= 200, "expected the shipped maps, got {n}");
    let (d, tier) = keeporig1.expect("keeporig map00001 analysed");
    assert_ne!(
        d,
        kfx_analysis::nav::INF,
        "keeporig 1: the heart reaches the portal by digging"
    );
    assert!(matches!(tier, Tier::Walk | Tier::Dig));
    // incremental update on the largest map
    if let Some(m) = largest {
        let mut a = Analysis::of_doc(&m.doc, &m.gd);
        let full = a.timings.total_us;
        let t0 = Instant::now();
        a.update(
            &m.doc,
            &m.gd,
            kfx_map::autotile::SlabRect {
                x0: 10,
                y0: 10,
                x1: 14,
                y1: 14,
            },
        );
        println!(
            "largest map {}x{} ({} keepers): full analysis {:.1} ms (fields {:.1}, economy {:.1}, \
                heroes+fairness {:.1}, symmetry {:.1}); update after a 5x5 edit {:.1} ms",
            m.doc.dims.w,
            m.doc.dims.h,
            a.fields.len(),
            full as f64 / 1000.0,
            a.timings.fields_us as f64 / 1000.0,
            a.timings.economy_us as f64 / 1000.0,
            a.timings.heroes_us as f64 / 1000.0,
            a.timings.symmetry_us as f64 / 1000.0,
            t0.elapsed().as_micros() as f64 / 1000.0
        );
    }
}

/// Overview + territory images of two symmetric multiplayer maps and one that is not.
#[test]
fn render_spot_checks() {
    if kfx_formats::testutil::install_dir().is_none() {
        return;
    }
    let mut picked: Vec<(String, bool)> = vec![];
    common::for_each_shipped(|m| {
        if picked.len() >= 3 {
            return;
        }
        let a = Analysis::of_doc(&m.doc, &m.gd);
        let is_mp = m
            .folder
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("multiplayer"));
        if a.sites.keepers().len() < 2 || !is_mp {
            return;
        }
        let sym = a.symmetry.iter().any(|c| c.terrain_exact());
        let want_sym = picked.iter().filter(|p| p.1).count() < 2;
        let want_asym = !picked.iter().any(|p| !p.1);
        if (sym && want_sym) || (!sym && want_asym) {
            let name = format!(
                "{}_map{:05}",
                m.folder.file_name().unwrap().to_string_lossy(),
                m.level
            );
            let dir = common::out_dir();
            common::write_png(
                &dir.join(format!("spot_{name}_overview.png")),
                &render_overview(&a, 5),
            );
            common::write_png(
                &dir.join(format!("spot_{name}_territory.png")),
                &render_layer(&a.terrain, &territory_layer(&a), 5, 140),
            );
            println!(
                "{name}: symmetric {sym}: {:?}",
                a.symmetry
                    .iter()
                    .map(|c| (c.mode, c.slab_breaks.len(), c.thing_breaks.len()))
                    .collect::<Vec<_>>()
            );
            if let Some(f) = &a.fairness {
                for r in &f.rows {
                    println!("  {:?}", r);
                }
                println!("  score {:.3}: {:?}", f.score, f.top(3));
            }
            picked.push((name, sym));
        }
    });
}
