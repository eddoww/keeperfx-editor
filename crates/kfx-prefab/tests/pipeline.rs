mod common;
use common::*;
use kfx_map::autotile::SlabRect;
use kfx_map::patch::DocState;
use kfx_map::things::{self, class, thing_class};
use kfx_prefab::*;

#[test]
fn capture_apply_round_trip_and_undo() {
    let Some(gd) = gd() else { return };
    let src = source(gd);
    let mut opts = CaptureOptions {
        name: "test".into(),
        ..Default::default()
    };
    opts.script_dk = Some(
        "IF_ACTION_POINT(7, PLAYER1)\n\tADD_PARTY_TO_LEVEL(PLAYER_GOOD, X, -9, 1, 1)\nENDIF\n"
            .into(),
    );
    let p = capture(
        &src,
        gd,
        &Selection::Rect(SlabRect {
            x0: 4,
            y0: 4,
            x1: 14,
            y1: 12,
        }),
        &opts,
    )
    .unwrap();
    assert_eq!((p.w, p.h), (11, 9));
    assert!(
        p.cells
            .iter()
            .flatten()
            .any(|c| c.slab == "HEART_PEDESTAL" && c.owner == model::Owner::Slot(0)),
        "player 1 became slot A"
    );
    assert!(p
        .things
        .iter()
        .any(|t| t.class == model::ThingClass::Door && t.door_locked == Some(true)));
    assert!(
        p.things
            .iter()
            .all(|t| t.model != "TORCH" && t.model != "SOUL_CONTAINER"),
        "slab objects are not captured"
    );
    assert!(
        p.things.iter().any(|t| t.model == "IMP") == false,
        "the imp stands outside the rect"
    );
    // the file round trips
    let text = p.to_toml();
    let back = Prefab::from_toml(&text).unwrap();
    assert_eq!(back.to_toml(), text);

    // apply onto a blank map, every orientation, undo restores exactly
    for rot in Rotation::ALL {
        for flip in [false, true] {
            let mut dst = blank(gd, 50, 40);
            let before = DocState::of(&dst);
            let o = ApplyOptions {
                orientation: Orientation {
                    rotation: rot,
                    flip,
                },
                owners: OwnerMap::with_first(&[2]),
                ..Default::default()
            };
            let (patch, rep) = apply(&mut dst, gd, &p, (20, 15), &o).unwrap();
            assert!(rep.slabs_written > 50, "{}", rep.summary());
            assert!(!rep.has_missing());
            assert_eq!(rep.things_added, p.things.len());
            // owner slot A is player 2 now
            assert!(dst.owners.iter().any(|&o| o == 2) && !dst.owners.iter().any(|&o| o == 1));
            // the door stands, its slab is a door slab, and the document validates
            let door = dst
                .things
                .items
                .iter()
                .position(|t| thing_class(t) == Some(class::DOOR))
                .expect("door");
            let (sx, sy) =
                things::position(dst.things.items[door].x, dst.things.items[door].y).unwrap();
            assert!(
                dst.door_can_stand(gd, sx / 3, sy / 3),
                "door can stand ({rot:?} flip {flip})"
            );
            let issues: Vec<_> = kfx_map::validate::validate(&dst, gd)
                .into_iter()
                .filter(|i| i.severity >= kfx_map::validate::Severity::Warning)
                .collect();
            // the only warning allowed: the copied heart belongs to player 2 and is there
            assert!(
                issues
                    .iter()
                    .all(|i| i.severity == kfx_map::validate::Severity::Error
                        || i.code == "thing-unreachable"),
                "{issues:?}"
            );
            let errors: Vec<_> = issues
                .iter()
                .filter(|i| i.severity == kfx_map::validate::Severity::Error)
                .collect();
            assert!(errors.is_empty(), "{errors:?}");
            // script fragment rewritten with the new numbers
            let ap = dst.action_points.items[0].number.unwrap();
            let gate = dst
                .things
                .items
                .iter()
                .find(|t| things::is_hero_gate(gd, t))
                .unwrap()
                .herogate_number
                .unwrap();
            let s = rep.script_dk.unwrap();
            assert!(
                s.contains(&format!("IF_ACTION_POINT({ap}, PLAYER2)")),
                "{s}"
            );
            assert!(s.contains(&format!("-{gate}")), "{s}");
            // undo
            patch.revert(&mut dst);
            assert_eq!(
                DocState::of(&dst),
                before,
                "undo is exact ({rot:?} flip {flip})"
            );
            patch.apply(&mut dst);
            assert!(dst.owners.iter().any(|&o| o == 2));
        }
    }
}
