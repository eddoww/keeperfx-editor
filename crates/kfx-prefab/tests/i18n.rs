use kfx_prefab::i18n::*;

#[test]
fn the_shipped_english_catalogue_parses() {
    let c = Catalog::parse(ENGLISH).unwrap();
    assert_eq!(c.code, "en");
    assert!(c.entries.len() > 500, "{} entries", c.entries.len());
    assert!(c
        .entries
        .values()
        .all(|e| (!e.text.is_empty() || e.other.is_some()) && !e.src.is_empty()));
    // keys are `<crate>.<slug>[_hash]`
    assert!(c.entries.keys().all(|k| k
        .split_once('.')
        .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty())));
    // tr() answers from the compiled-in catalogue
    let (key, e) = c.entries.iter().find(|(_, e)| !e.text.is_empty()).unwrap();
    set_language("en");
    assert_eq!(tr(key), e.text);
}
