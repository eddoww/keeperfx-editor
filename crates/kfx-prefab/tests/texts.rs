//! The translation workflow on a private copy of a shipped campaign (`ancntkpr`, which has its
//! own strings files in seven languages). Nothing in the install is written.
use kfx_formats::strings::{Lang, StringResolver, StringSources};
use kfx_prefab::texts::*;
use std::path::PathBuf;

struct Copy {
    root: PathBuf,
    install: PathBuf,
}

impl Copy {
    fn new(tag: &str) -> Option<Copy> {
        let install = kfx_formats::testutil::install_dir()?;
        let root = kfx_formats::testutil::scratch_dir("prefab")
            .join(format!("texts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let camp = root.join("campgns/ancntkpr");
        std::fs::create_dir_all(&camp).ok()?;
        let src = kfx_formats::testutil::resolve_ci(&install, "campgns")?;
        std::fs::copy(src.join("ancntkpr.cfg"), root.join("campgns/ancntkpr.cfg")).ok()?;
        for e in std::fs::read_dir(src.join("ancntkpr")).ok()?.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            let keep = n.ends_with(".txt")
                || n.ends_with(".lif")
                || n.starts_with("text_")
                || n.ends_with(".lua");
            if keep && e.path().is_file() {
                std::fs::copy(e.path(), camp.join(&n)).ok()?;
            }
        }
        kfx_formats::testutil::symlink(
            &kfx_formats::testutil::resolve_ci(&install, "fxdata")?,
            &root.join("fxdata"),
        )
        .ok()?;
        Some(Copy { root, install })
    }
    fn spec(&self) -> ProjectSpec {
        ProjectSpec::for_campaign(
            &self.root,
            &self.root.join("campgns/ancntkpr.cfg"),
            Some((1..=6).collect()),
        )
        .unwrap()
    }
    fn resolver(&self, level: u32, lang: Lang) -> StringResolver {
        let spec = self.spec();
        let s = StringSources::resolve(
            &self.root,
            &[],
            &spec.campaign_strings,
            Some(&spec.levels_dir),
            Some(level),
            lang,
        )
        .unwrap();
        StringResolver::load(&s).unwrap()
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        // only the private copy (a symlink to fxdata is removed as a link, never followed)
        if self
            .root
            .starts_with(kfx_formats::testutil::scratch_dir("prefab"))
        {
            let _ = std::fs::remove_file(self.root.join("fxdata"));
            let _ = std::fs::remove_dir_all(&self.root);
        }
        let _ = &self.install;
    }
}

#[test]
fn collects_ids_and_reads_what_the_game_reads() {
    let Some(c) = Copy::new("read") else { return };
    let p = TextProject::open(c.spec()).unwrap();
    assert!(p.rows.len() >= 10, "{} rows", p.rows.len());
    println!(
        "{} rows, {} ignored ids, languages {:?}",
        p.rows.len(),
        p.ignored.len(),
        p.langs.iter().map(|l| l.code()).collect::<Vec<_>>()
    );
    assert!(p.rows.iter().any(|r| r
        .refs
        .iter()
        .any(|x| x.kind == RefKind::Script && x.line > 0)));
    // every cell the grid shows equals what the game's resolver answers
    let mut checked = 0;
    for (ri, row) in p.rows.iter().enumerate() {
        let level = match row.scope {
            Scope::Level(n) => n,
            Scope::Campaign => row.refs[0].level,
        };
        for &l in &p.langs {
            let cell = p.cell(ri, l);
            if matches!(cell.state, CellState::Missing) {
                continue;
            }
            let r = c.resolver(level, l);
            assert_eq!(
                cell.text,
                r.get(row.id),
                "level {level} id {} {}",
                row.id,
                l.code()
            );
            checked += 1;
        }
    }
    println!("{checked} cells equal the resolver's answer");
    assert!(checked > 20);
    for &l in &p.langs {
        let s = p.stats(l);
        println!("{} {:?}", l.code(), s);
        assert_eq!(
            s.translated + s.copied + s.outdated + s.missing + s.inherited,
            s.total
        );
    }
}

#[test]
fn edit_save_outdated_copy_and_create_campaign_file() {
    let Some(c) = Copy::new("edit") else { return };
    let mut p = TextProject::open(c.spec()).unwrap();
    let ri = p
        .rows
        .iter()
        .position(|r| {
            r.scope == Scope::Campaign
                && !p
                    .source_text(p.row_index(r.scope, r.id).unwrap())
                    .is_empty()
        })
        .expect("a campaign row");
    let id = p.rows[ri].id;
    let level = p.rows[ri].refs[0].level;
    // encodability: euro sign is not in the French code page, Japanese is not either for FRE
    assert!(matches!(
        p.set_text(ri, Lang::French, "prix 5 \u{20ac}"),
        Err(SetError::Unencodable(_))
    ));
    assert!(matches!(
        p.set_text(ri, Lang::French, "a\0b"),
        Err(SetError::ContainsNul)
    ));
    assert!(TextProject::check_encodable(Lang::Japanese, "\u{65e5}\u{672c}").is_ok());
    // a good translation goes to the campaign's French file, nowhere else
    let target = p.write_target(ri, Lang::French).unwrap();
    assert!(
        target.starts_with(&c.root) && target.ends_with("text_fre.dat"),
        "{target:?}"
    );
    p.set_text(ri, Lang::French, "Bonjour \u{e9}t\u{e9}, ma\u{ee}tre")
        .unwrap();
    assert!(p.is_dirty());
    let written = p.save().unwrap();
    assert!(
        written.iter().any(|w| w.ends_with("text_fre.dat"))
            && written
                .iter()
                .any(|w| w.to_string_lossy().contains("kfxtexts")),
        "{written:?}"
    );
    assert_eq!(
        c.resolver(level, Lang::French).get(id),
        "Bonjour \u{e9}t\u{e9}, ma\u{ee}tre",
        "the game's resolver reads the new text"
    );
    // outdated: the source changes after the translation was made
    assert_eq!(p.cell(ri, Lang::French).state, CellState::Ok);
    let old_src = p.source_text(ri);
    p.set_text(ri, Lang::English, &format!("{old_src} (edited)"))
        .unwrap();
    assert_eq!(p.cell(ri, Lang::French).state, CellState::Outdated);
    p.save().unwrap();
    let mut q = TextProject::open(c.spec()).unwrap();
    assert_eq!(
        q.cell(ri, Lang::French).state,
        CellState::Outdated,
        "outdated survives a reload (sidecar)"
    );
    assert_eq!(q.stats(Lang::French).outdated, 1);
    q.mark_current(ri, Lang::French);
    assert_eq!(q.cell(ri, Lang::French).state, CellState::Ok);
    // a new language: the campaign has no Dutch file, campaign rows cannot be written yet
    q.add_language(Lang::Dutch);
    assert!(q.write_target(ri, Lang::Dutch).is_err());
    let r0 = q.copy_source_to_missing(Some(&[Lang::Dutch]));
    assert_eq!(r0.copied, 0);
    assert!(!r0.no_target.is_empty());
    let dut = q.create_campaign_file(Lang::Dutch).unwrap();
    assert!(dut.exists() && std::fs::metadata(&dut).unwrap().len() >= 16);
    assert!(std::fs::read_to_string(c.root.join("campgns/ancntkpr.cfg"))
        .unwrap()
        .contains("DUT = campgns/ancntkpr/text_dut.dat"));
    let r1 = q.copy_source_to_missing(Some(&[Lang::Dutch]));
    println!("copy source to missing: {r1:?}");
    assert!(r1.copied > 0);
    let s = q.stats(Lang::Dutch);
    assert_eq!(s.copied, r1.copied);
    q.save().unwrap();
    let again = TextProject::open(c.spec());
    assert!(again.is_ok());
    // the copy is visible to the game (the resolver picks the Dutch campaign file once listed)
    let src_text = q.source_text(ri);
    let spec = c.spec();
    assert!(spec.campaign_strings.iter().any(|(code, _)| code == "DUT"));
    assert_eq!(c.resolver(level, Lang::Dutch).get(id), src_text);
}

#[test]
fn level_texts_go_to_the_level_file() {
    let Some(c) = Copy::new("level") else { return };
    let lvl = c.root.join("campgns/ancntkpr/map00001.txt");
    let mut s = std::fs::read_to_string(&lvl).unwrap();
    s.push_str("\nDISPLAY_INFORMATION(1999)\n");
    std::fs::write(&lvl, s).unwrap();
    let mut p = TextProject::open(c.spec()).unwrap();
    let ri = p
        .row_index(Scope::Level(1), 1999)
        .expect("new id is a level text");
    assert_eq!(p.cell(ri, Lang::English).state, CellState::Missing);
    p.set_text(ri, Lang::English, "A brand new message")
        .unwrap();
    p.set_text(ri, Lang::German, "Eine ganz neue Nachricht: \u{fc}\u{df}")
        .unwrap();
    // before the save the grid already shows the edits (the files do not exist yet)
    assert_eq!(
        p.cell(ri, Lang::German).text,
        "Eine ganz neue Nachricht: \u{fc}\u{df}"
    );
    assert_eq!(p.cell(ri, Lang::English).state, CellState::Source);
    p.save().unwrap();
    assert!(c.root.join("campgns/ancntkpr/map00001.eng.dat").is_file());
    assert!(
        std::fs::metadata(c.root.join("campgns/ancntkpr/map00001.ger.dat"))
            .unwrap()
            .len()
            >= 16
    );
    assert_eq!(
        c.resolver(1, Lang::English).get(1999),
        "A brand new message"
    );
    assert_eq!(
        c.resolver(1, Lang::German).get(1999),
        "Eine ganz neue Nachricht: \u{fc}\u{df}"
    );
    // a language without a level file gets the campaign's default language file, like the game
    assert_eq!(c.resolver(1, Lang::French).get(1999), "A brand new message");
    let ri = p.row_index(Scope::Level(1), 1999).unwrap();
    assert_eq!(
        p.cell(ri, Lang::French).state,
        CellState::Missing,
        "no translation of its own, the game shows English"
    );
}

#[test]
fn csv_and_po_round_trip() {
    let Some(c) = Copy::new("exchange") else {
        return;
    };
    let mut p = TextProject::open(c.spec()).unwrap();
    let langs = p.langs.clone();
    let csv = p.to_csv(&langs);
    // exporting and importing the same file changes nothing
    let before: Vec<Vec<String>> = (0..p.rows.len())
        .map(|ri| langs.iter().map(|&l| p.cell(ri, l).text).collect())
        .collect();
    let rep = p.import_csv(&csv).unwrap();
    assert!(
        rep.updated.is_empty() && rep.errors.is_empty() && rep.unknown.is_empty(),
        "{rep:?}"
    );
    assert!(rep.unchanged > 0);
    assert!(!p.is_dirty(), "nothing to save after a no-op import");
    // the CSV parser round-trips awkward text
    let records = csv::parse(&csv).unwrap();
    assert_eq!(records.len(), p.rows.len() + 1);
    assert!(records.iter().all(|r| r.len() == 3 + langs.len()));
    // a translator edits one French cell: quotes, comma, line break, accents
    let ri = p
        .rows
        .iter()
        .position(|r| r.scope == Scope::Campaign)
        .unwrap();
    let (scope, id) = (p.rows[ri].scope, p.rows[ri].id);
    let tricky = "Il dit \"all\u{e9}\", puis\nse tait, a\u{ef}e";
    let col = 3 + langs.iter().position(|l| *l == Lang::French).unwrap();
    let mut recs = records.clone();
    let row = recs
        .iter_mut()
        .find(|r| r[0] == scope.key() && r[1] == id.to_string())
        .unwrap();
    row[col] = tricky.to_string();
    let edited: String = recs
        .iter()
        .map(|r| r.iter().map(|f| csv_quote(f)).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    let rep = p.import_csv(&edited).unwrap();
    assert_eq!(rep.updated, vec![(scope, id, Lang::French)], "{rep:?}");
    assert_eq!(p.cell(ri, Lang::French).text, tricky);
    // unencodable text is reported, not written
    let mut recs2 = records.clone();
    recs2
        .iter_mut()
        .find(|r| r[0] == scope.key() && r[1] == id.to_string())
        .unwrap()[col] = "euro \u{20ac}".into();
    let bad: String = recs2
        .iter()
        .map(|r| r.iter().map(|f| csv_quote(f)).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n");
    let rep = p.import_csv(&bad).unwrap();
    assert_eq!(rep.errors.len(), 1, "{rep:?}");

    // .po: export French, change nothing -> no update; edit, mark fuzzy -> imported but outdated
    let po = p.to_po(Lang::French);
    assert!(po.contains("X-KFX-Language: FRE") && po.contains("msgctxt \"campaign:"));
    let rep = p.import_po(Lang::French, &po).unwrap();
    assert!(rep.updated.is_empty() && rep.errors.is_empty(), "{rep:?}");
    let ctx = format!("msgctxt \"{}:{}\"", scope.key(), id);
    let i = po.find(&ctx).unwrap();
    let end = i + po[i..].find("\n\n").unwrap_or(po.len() - i);
    let entry = &po[i..end];
    let msgid_start = entry.find("msgid ").unwrap();
    let new_entry = format!(
        "{}msgid {}",
        &entry[..msgid_start].replace("#, fuzzy\n", ""),
        {
            let rest = &entry[msgid_start + 6..];
            let ms = rest.find("\nmsgstr").unwrap();
            format!(
                "{}\nmsgstr \"\"\n\"Premi\\u00e8re ligne\\n\"\n\"Deuxi\\\"eme\"",
                &rest[..ms]
            )
            .replace("\\u00e8", "\u{e8}")
        }
    );
    let _ = new_entry;
    // simpler and exact: rebuild the entry by hand with the same msgctxt / msgid
    let src = p.source_text(ri);
    let mut po2 = String::from("msgid \"\"\nmsgstr \"\"\n\"X-KFX-Language: FRE\\n\"\n\n");
    po2.push_str(&format!(
        "#, fuzzy\nmsgctxt \"{}:{}\"\nmsgid \"{}\"\nmsgstr \"\"\n\
            \"Premi\u{e8}re ligne\\n\"\n\"Deuxi\\\"eme, \u{e0} toi\"\n",
        scope.key(),
        id,
        src.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    ));
    let rep = p.import_po(Lang::French, &po2).unwrap();
    assert_eq!(rep.updated, vec![(scope, id, Lang::French)], "{rep:?}");
    assert_eq!(
        p.cell(ri, Lang::French).text,
        "Premi\u{e8}re ligne\nDeuxi\"eme, \u{e0} toi"
    );
    assert_eq!(
        p.cell(ri, Lang::French).state,
        CellState::Outdated,
        "a fuzzy entry stays outdated"
    );
    // a po of unchanged texts after the exchange: export again and import into a fresh project
    let mut fresh = TextProject::open(c.spec()).unwrap();
    let po3 = p.to_po(Lang::French);
    let rep = fresh.import_po(Lang::French, &po3).unwrap();
    assert_eq!(
        rep.updated,
        vec![(scope, id, Lang::French)],
        "only the edited cell differs: {rep:?}"
    );
    assert_eq!(
        fresh.cell(ri, Lang::French).text,
        p.cell(ri, Lang::French).text
    );
    let _ = before;
}

fn csv_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
