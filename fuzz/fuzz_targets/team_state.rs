#![no_main]
//! The team battle's decoded state (`fight::team`, as a save state, a rollback snapshot or a replay's checkpoint
//! carries it): the input's first byte picks the mode (bit 0: 2 VS 2 / FREE FOR ALL) and the simulation version
//! (bit 1: v22 / v24), the rest is postcard bytes. A `TeamFight` must decode to a value or an Err (no panic, no huge
//! allocation; `rebind` needs the static data, so its `check` runs in the engine's tests); a `TeamRound` that `check`
//! accepts must run `tick` for frames on four blank fighters (fresh ones and `start` at each next round, as
//! `TeamFight`) without a panic (an index, an overflow), and the targets `target::update` keeps (with the thrown
//! fighters' `keep` of `target::thrown`, the locks decoded as `target::Locks` from the bytes after the
//! `TeamRound`: the custom deserialiser must refuse a thrower or a frame count out of range) must stay enemies.
use open_kumite_beast::fight::fighter::Fighter;
use open_kumite_beast::fight::team::round::Request;
use open_kumite_beast::fight::team::{N, TeamFight, TeamMode, TeamRound, target};
use open_kumite_beast::fight::version::{Gates, v};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&m, bytes)) = data.split_first() else { return };
    let mode = if m & 1 == 0 { TeamMode::Vs2 } else { TeamMode::Ffa };
    let sim = Gates::of(if m & 2 == 0 { v::TEAM } else { v::SUDDEN_DEATH_DRAW });
    let _ = postcard::from_bytes::<TeamFight>(bytes);
    let Ok((mut r, rest)) = postcard::take_from_bytes::<TeamRound>(bytes) else { return };
    if r.check(mode).is_err() {
        return;
    }
    let mut fs: [Fighter; N] = std::array::from_fn(|k| Fighter::new((k & 1) as u8, 0));
    let team: [usize; N] = std::array::from_fn(|k| mode.team(k));
    let locks = postcard::from_bytes::<target::Locks>(rest).unwrap_or_default();
    let (mut tgt, mut locked, mut thrown_by) = ([1, 0, 3, 2], locks.l2, locks.thrown_by);
    let mut sounds = Vec::new();
    for f in 0..600u32 {
        // a fighter KO'd now and then (F.0 bit 7), START / □ pressed on some frames
        if let Some(&b) = bytes.get(f as usize % bytes.len().max(1))
            && b & 0xF0 == 0xF0
        {
            fs[(b & 3) as usize].o_00 |= 0x80;
        }
        let pressed = if f % 7 == 0 { 0x0808 } else { 0 };
        if let Request::NextRound(_) = r.tick(&mut fs, mode, &mut sounds, pressed, sim, None) {
            // as `TeamFight`: fresh fighters, the round restarted
            fs = std::array::from_fn(|k| Fighter::new((k & 1) as u8, 0));
            r.start();
        }
        // a thrown fighter done (lying, getting up or standing) on some frames
        let done: [bool; N] = std::array::from_fn(|k| bytes.get((f as usize + k) % bytes.len().max(1)).is_some_and(|&b| b & 0x0F == 0x0F));
        let keep = target::thrown(&fs, &tgt, &mut thrown_by, done, &r.out);
        target::update(&fs, &mut tgt, &mut locked, &team, &r.out, [f % 5 == 0, false, f % 3 == 0, false], keep);
        for k in 0..N {
            assert!(tgt[k] < N && team[tgt[k]] != team[k]);
        }
        sounds.clear();
    }
});
