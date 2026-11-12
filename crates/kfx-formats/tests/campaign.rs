use kfx_formats::campaign::*;
use kfx_formats::levelinfo::*;
use kfx_formats::testutil::install_dir;

fn cfgs(root: &std::path::Path, sub: &str) -> Vec<std::path::PathBuf> {
    let dir = kfx_formats::testutil::resolve_ci(root, sub).unwrap();
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cfg")))
        .collect();
    v.sort();
    v
}

#[test]
fn resolve_all_install_packs() {
    let Some(root) = install_dir() else { return };
    let mut total = 0;
    let mut total_levels = 0;
    for sub in ["campgns", "levels", "multiplayer"] {
        for p in cfgs(&root, sub) {
            let raw = std::fs::read(&p).unwrap();
            let c = Campaign::parse(&raw);
            assert_eq!(c.to_bytes(), raw);
            // re-applying the parsed [common] changes nothing
            let mut c2 = c.clone();
            c2.set_common(&c.common());
            assert_eq!(c2.to_bytes(), raw, "{}", p.display());
            let r = resolve(&root, &p).unwrap();
            let sizes: std::collections::BTreeMap<_, usize> =
                r.levels.iter().fold(Default::default(), |mut m, l| {
                    *m.entry(l.mapsize).or_default() += 1;
                    m
                });
            let missing_slb = r.levels.iter().filter(|l| l.slb.is_none()).count();
            println!(
                "{:?} {:<14} name={:?} levels={} levels_loc={:?} (dir exists: {}) configs={} \
                    creatures={} sizes={:?} without_slb={}",
                r.kind,
                p.file_name().unwrap().to_string_lossy(),
                r.name,
                r.levels.len(),
                r.common.levels_location,
                r.levels_dir.is_some(),
                r.configs_dir.is_some(),
                r.creatures_dir.is_some(),
                sizes,
                missing_slb
            );
            total += 1;
            total_levels += r.levels.len();
        }
    }
    println!("campaign cfgs resolved: {total}, levels: {total_levels}");
    assert!(total > 0);
}

#[test]
fn typed_edit() {
    let src = b"; x\r\n[common]\r\nNAME = Test\r\nSINGLE_LEVELS = 1 2\r\nBONUS_LEVELS = 0 \
        5\r\n\r\n[strings]\r\nENG = a.dat\r\n\r\n[map00001]\r\nNAME_TEXT = One\r\nMAPSIZE = 85 \
        85\r\n";
    let mut c = Campaign::parse(src);
    let mut cm = c.common();
    assert_eq!(cm.single_levels, vec![1, 2]);
    assert_eq!(cm.bonus_levels, vec![0, 5]);
    cm.name = Some("New".into());
    cm.single_levels.push(3);
    cm.levels_location = Some("campgns/test".into());
    c.set_common(&cm);
    let mut i = c.map_info(1).unwrap();
    i.mapsize = Some((101, 101));
    c.set_map_info(1, &i);
    c.set_map_info(
        2,
        &LevelInfo {
            name_text: Some("Two".into()),
            ..Default::default()
        },
    );
    let t = String::from_utf8(c.to_bytes()).unwrap();
    println!("{t}");
    assert_eq!(c.common().single_levels, vec![1, 2, 3]);
    assert_eq!(c.common().levels_location.as_deref(), Some("campgns/test"));
    assert_eq!(c.map_info(1).unwrap().mapsize, Some((101, 101)));
    assert_eq!(c.map_info(2).unwrap().name_text.as_deref(), Some("Two"));
    assert_eq!(c.map_block_numbers(), vec![1, 2]);
    assert_eq!(c.strings(), vec![("ENG".to_string(), "a.dat".to_string())]);
    assert!(t.contains("[strings]\r\nENG = a.dat\r\n"));
}
