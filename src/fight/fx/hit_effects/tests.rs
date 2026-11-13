//! The hit effects' tests.

use super::*;

/// The tables of the export next to the engine (OKB_EXPORT overrides); None skips the test.
fn tables() -> Option<Tables> {
    crate::formats::tables::export_tables()
}

/// The beast revert's emitter (`emitter_spawn` mode 0x83, a timeline of one spawn, then back to
/// the start six times), frame by frame: the most kind-14 sparks and kind-15 trails alive at once, and the
/// frames until the last task is gone.
fn revert_sparks(keep_slot: bool) -> (usize, usize, usize) {
    let tables = tables().expect("export");
    let mut fx = Fx::new(Arc::from(Vec::new()), Arena { half: (0x700, 0x700) });
    fx.keep_slot = keep_slot;
    fx.emitter_spawn([0, -800, 0], None, 0xE, 0x83, tables.table(crate::fight::fx::EMITTER_TIMELINES).origin(), [0; 4]);
    let (mut most, mut frames) = ([0; 2], 0);
    while fx.live() > 0 && frames < 200 {
        fx.tick(&tables);
        frames += 1;
        for (k, kind) in [14, 15].into_iter().enumerate() {
            let n = fx.tasks.iter().flatten().filter(|t| matches!(&t.kind, Kind::Sprite(s) if s.kind == kind)).count();
            most[k] = most[k].max(n);
        }
    }
    (most[0], most[1], frames)
}

/// A task that spawns while it runs keeps its slot (simulation version 6): the seven sparks with their trails,
/// about 40 frames as in the game; up to version 5 the timeline and each spark lost their slot to the first
/// task they spawned.
#[test]
fn spawning_task_keeps_its_slot() {
    if tables().is_none() {
        eprintln!("skipped: no okb_export");
        return;
    }
    // 7 sparks, 4 trails each (every third frame of 12), the last trail gone after about 40 frames
    let (sparks, trails, frames) = revert_sparks(true);
    assert_eq!((sparks, trails), (7, 28));
    assert!((36..=44).contains(&frames), "{frames} frames");
    let (sparks, trails, frames) = revert_sparks(false);
    assert_eq!((sparks, trails), (1, 1));
    assert!(frames < 30, "{frames} frames");
}

/// The effects' rsin is the portable one over the whole circle (`portable::tests::rsin_table` checks the
/// portable one's margins).
#[test]
fn rsin_is_portable() {
    assert!((0..4096).all(|a| rsin(a) == crate::base::portable::rsin(a)));
}
