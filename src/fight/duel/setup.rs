//! What a fight of either kind sets up and checks per fighter: a fresh fighter, the effect VM's groups, the hit-spark
//! ring tables, the decoded state's checks.
//!
//! State: none of its own: builds and checks parts of the serialised `Match` / `TeamFight`
//!
//! Free functions shared by the match (`fight::sim`) and the team battle (`fight::team`, engine addition), which
//! build their worlds from them.

use crate::assets::roster::CHARACTERS;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F6C, Fighter};
use crate::fight::input;
use crate::fight::move_decision::World;
use crate::fight::move_script;

/// Side `i`'s fighter at a round start (`Match::fresh_fighters` for both; `prev`: its fighter before; player `i & 1`:
/// the team battle's slot, which places its fighters itself).
pub(crate) fn fresh_fighter(data: &Loaded, i: usize, mode: u8, prev: Option<&Fighter>, hp_pct: i32) -> Fighter {
    let p = crate::fight::side::Fid::of(i).slot().index() as u8;
    let chr = CHARACTERS.iter().position(|c| *c == data.fighters[i].code).unwrap_or(0) as u8;
    let mut f = Fighter::new(p, chr);
    input::reset_buttons(&mut f);
    f.pos = [if i == 0 { -0x100 } else { 0x100 }, 0, 0];
    f.m_t = [f.pos[0] as i32, 0, 0];
    f.m_pos = f.pos;
    f.yaw_base = if i == 0 { 0x400 } else { -0x400 };
    f.yaw = f.yaw_base;
    let ani = &data.fighters[i].ani;
    f.o_b0 = (ani.param(5) as i32 * hp_pct / 100) as i16; // f.DE: ATTACK LEVEL HP %
    f.o_fc = ani.param(7);
    f.o_fe = ani.param(8);
    f.o_100 = ani.param(9);
    f.o_102 = ani.param(10);
    f.o_104 = ani.param(11);
    f.o_24 = ani.param(2);
    // F54 bits 0-1 = the player's control options (settings +0x19 / +0x1A); bit 0 enables guarding (on)
    f.flags[crate::fight::fighter::F54] = 1;
    f.gauge = match (mode, prev) {
        (2, Some(pf)) => pf.gauge.max(0),
        _ => 0x1E00,
    };
    if let (4, Some(pf)) = (mode, prev) {
        // the fighter reset, mode 4: a Beast Drive (F6C bit 15) ends the beast form and empties the gauge; the reload
        // reloads the ANI values (F.B0 unscaled: the value load in mode 4 returns before the ATTACK LEVEL scaling)
        // and keeps the damage F.AA / AC / AE, the gauge F.D6 / D8, F.1 and F.54
        let p = pf;
        let drive = p.flags[F6C] & 0x8000 != 0;
        (f.gauge, f.o_d8) = if drive { (0, 0) } else { (p.gauge, p.o_d8) };
        f.form = if !drive && p.form & 0x10 != 0 { 0x11 } else { 0 };
        (f.o_aa, f.o_ac, f.o_ae) = (p.o_aa, p.o_ac, p.o_ae);
        f.o_b0 = ani.param(5);
        f.o_01 = p.o_01;
        f.flags[crate::fight::fighter::F54] = p.flags[crate::fight::fighter::F54];
        f.node_scale = p.node_scale.clone();
        f.move_req = p.move_req;
    }
    if let (2, Some(pf)) = (mode, prev) {
        if pf.form & 0x10 != 0 {
            f.form = 0x11; // a transformed fighter starts the next round in beast form
        }
        f.o_01 = pf.o_01 & 0x7F;
    }
    if mode == 3 {
        f.o_ac = f.o_b0; // sudden death: the next hit decides
        f.flags[F5C] |= 0x20_0000;
    }
    f.o_ce = 0x1E0;
    if mode != 4 {
        f.move_req = 0x28;
    }
    // the fighter reset ends with a move start (F, S, S.80)
    let mv = f.move_req;
    move_script::start_move(&mut f, ani, mv);
    f
}

/// The effect VM with the effect reset's script groups: ScriptGroup = &EffTab[80 / 50 / 70 / 35].
/// The groups reach to the end of the match's 100 entries only: the team battle's bank 1 entries 100.. are its
/// own groups 5 / 6, and a group past 64 slots runs into the next one's addresses (`Vm::set_group`; group 3's 65th
/// only into the unused group 4).
pub(crate) fn effect_vm(data: &Loaded) -> crate::fight::fx::script_vm::Vm {
    let mut vm = crate::fight::fx::script_vm::Vm::new();
    vm.portable = data.portable();
    let tab = &data.eff_tab[..data.eff_tab.len().min(100)];
    for (g, b) in [(0usize, 80usize), (1, 50), (2, 70), (3, 35)] {
        vm.set_group(g, &tab[b..]);
    }
    // the UV animator tables of model_fx (the static block: uv deltas, script) as the native code reads them
    let block = data.tables.table(crate::fight::fx::STATIC_BLOCK);
    vm.mem.map_static(block.origin(), block.bytes().into());
    // EmitterGroup = &EffTab[0 / 40 / 60 / 30]
    for (g, b) in [(0usize, 0usize), (1, 40), (2, 60), (3, 30)] {
        vm.set_model_group(g, &tab[b..]);
    }
    vm
}

/// EFT.DAT entry 88, the hit-spark ring tables (`hit_effects::Fx`).
pub(crate) fn rings(data: &Loaded) -> std::sync::Arc<[u8]> {
    data.eft.get(88).cloned().flatten().unwrap_or_default().into()
}

/// The world's part of a decoded state's check (`Match::check`, also run on the KO replay's snapshots;
/// `TeamFight::check`; `br1` whether that state holds imported fighters): `stage::br1::Counters::sounds`, which
/// only simulation v18 on an imported stage without an imported fighter fills (a frame's, cleared each tick),
/// at most a queue's 32, no negative id.
pub(crate) fn check_world(world: &World, br1: bool, data: &Loaded) -> Result<(), String> {
    let sounds = &world.stage.br1.sounds;
    let may = data.gates().br1_debris && data.br1_stage.is_some() && !br1;
    if sounds.len() > if may { 32 } else { 0 } || sounds.iter().any(|&id| id < 0) {
        return Err("Imported stage sounds".into());
    }
    Ok(())
}

/// A main-game fighter's player, character, moves, move-script positions and attack chain fit its script file `ani`:
/// its player (F.02) P1 or P2 (the slot: the team battle's fighters 2 / 3 play as 0 / 1; `side::Slot::of_player` would
/// mask a larger one silently).
pub(crate) fn fighter_fits(f: &Fighter, ani: &crate::formats::act_ani::Ani) -> bool {
    let mv = |m: i16| (-1..ani.entries.len() as i64).contains(&(m as i64));
    let at = |p: usize| p <= ani.bytes.len();
    // the chain ring (+0x7C..+0x7E) indexes the 16 chain_moves
    let chain = f.chain_pos < 16 && f.chain_len < 16 && (-1..16).contains(&f.chain_sel);
    f.player < 2 && (f.chr as usize) < CHARACTERS.len() && mv(f.move_cur) && mv(f.move_req) && at(f.pc) && at(f.loop_mark) && at(f.ret) && chain
}
