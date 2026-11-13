//! The effect bytecode VM: its state, task pool, scheduler and effect models.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The effect bytecode VM (interpreter, scheduler list 4). Scripts address raw PS1
//! memory (their own object, script data, engine structures), so this port runs them in a small emulated address
//! space: script blobs, effect objects and engine-shared blocks are segments at fixed virtual addresses.
//! Task handles are addresses too (`HANDLE_BASE + 0x20·id`), as scripts store and pass them like pointers.
//!
//! Not here yet: the 3-D effect models (ED1C4, op0D / op03), which go through the [`Models`] hook.
//!
//! Here: the VM state, the task pool and scheduler (spawn, kill, `tick`, the destructors) and the live effect
//! models. `interp.rs` runs a task's bytecode, `ops.rs` / `anims.rs` / `timeline.rs` / `mat3.rs` hold the
//! instructions it hands on, `mem.rs` the address space.

use crate::fight::side::Bank;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

mod anims;
mod interp;
mod mat3;
mod mem;
mod ops;
#[cfg(test)]
mod tests;
mod timeline;

pub use mat3::M3;
pub use mem::Mem;

/// Virtual address layout.
const SCRIPT_BASE: u32 = 0x8100_0000;
const OBJ_BASE: u32 = 0x8800_0000;
const HANDLE_BASE: u32 = 0x9000_0000;

/// What the effect destructor does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Dtor {
    None,
    /// spawn_owned: clear the owner slot, run the script's cleanup entry hdr[4]
    Owned,
    /// fork with a slot: clear the slot only
    Slot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Kind {
    /// a VM context (the interpreter's task data)
    Script { base: u32, obj: u32, pc: u32, cond: i32, dp: u32, owns_obj: bool },
    /// model UV animator: relative uv_scroll steps from a script
    UvAnim { model: u32, slot: u32, uvtab: u32, base: u32, cur: u32, timer: i16, count: i16 },
    /// colour animator (spawned by op05.0E)
    Colour { ft4: [u32; 2], seq: u32, tab: u32, cur: u32, timer: i16, loops: i16, hold: i16, dur: i16, dir: i16 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Task {
    kind: Kind,
    slot: u32,
    dtor: Dtor,
    kill: bool,
}

/// One recorded billboard draw (op05.09 / 0A / 0B): world position in game units, size in world units
/// (the pixel size is `(w << 9) / otz` with otz = SZ / 4), the FT4 fields, a fixed OT slot for op05.0B.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Billboard {
    pub pos: [i32; 3],
    pub w: i32,
    pub fixed_otz: Option<i32>,
    pub uv: [[u8; 2]; 4],
    pub rgb: [u8; 3],
    pub tpage: u16,
    pub clut: u16,
    pub semi: bool,
}

/// A live 3-D effect model: the instance, the object slot that holds its handle and its record
/// `EmitterGroup[group][idx]` (`Vm::rebind`).
#[derive(Clone, Serialize, Deserialize)]
struct ModelSlot {
    inst: crate::fight::fx::effect_models::Instance,
    slot: u32,
    rec: (usize, usize),
}

/// How a VM run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
    Yield,
    End,
    Destroy,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Vm {
    pub mem: Mem,
    /// ScriptGroup[g]: script base addresses per group (0 = empty slot)
    groups: [Vec<u32>; 5],
    /// the team battle's script groups 5 / 6 (engine addition: fighters 2 / 3's `<CHR>EFT` scripts, `script_link::fx_group`);
    /// static, set with the others and put back by `rebind`
    #[serde(skip)]
    groups_ext: [Vec<u32>; 2],
    /// by handle (ordered, so the state serialises the same every time; never iterated)
    tasks: BTreeMap<u32, Task>,
    /// scheduler order, head first
    order: Vec<u32>,
    next_id: u32,
    next_obj: u32,
    seed: u32,
    /// the global pause flag (= match freeze +0x82)
    pub paused: bool,
    /// frame buffer index: which FT4 of a sprite block is drawn
    pub buf: usize,
    pub draws: Vec<Billboard>,
    /// EmitterGroup[g]: the ED1C4 records per group (static, `Vm::rebind`)
    #[serde(skip)]
    model_groups: [Vec<Option<Arc<crate::fight::fx::effect_models::Record>>>; 7],
    /// live models by handle (a VM address whose first word points at the model MATRIX), creation order
    models: Vec<(u32, ModelSlot)>,
    /// instructions left in this frame (guards against scripts that never yield)
    budget: u32,
    /// (op, sub) pairs met that are not implemented, for the debug report (not state)
    #[serde(skip)]
    pub unsupported: Vec<(u8, u8)>,
    /// spawn counts per (group, script) for the debug report (not state)
    #[serde(skip)]
    pub stats: BTreeMap<(usize, usize), u32>,
    /// simulation version 3 on (`Loaded::portable`): libm's atan2 (static, `Vm::rebind`)
    #[serde(skip)]
    pub portable: bool,
}

/// rsin / rcos on the 4096-unit circle (the sine table via the shared Trig in the engine; plain float here
/// rounded like the table: `portable::rsin`).
fn rsin(a: i32) -> i32 {
    crate::base::portable::rsin(a)
}
fn rcos(a: i32) -> i32 {
    rsin(a + 0x400)
}

impl Vm {
    pub fn new() -> Self {
        Self {
            mem: Mem::default(),
            groups: Default::default(),
            groups_ext: Default::default(),
            tasks: BTreeMap::new(),
            order: Vec::new(),
            next_id: 1,
            next_obj: OBJ_BASE,
            seed: 1,
            paused: false,
            buf: 0,
            draws: Vec::new(),
            model_groups: Default::default(),
            models: Vec::new(),
            budget: 0,
            unsupported: Vec::new(),
            stats: Default::default(),
            portable: false,
        }
    }

    /// The group a script at `base` means by group `g`: a script of the team battle's groups 5 / 6 (engine addition) that
    /// names P1's or P2's (1 / 2: the scripts pick their own `<CHR>EFT` entries by the side parameter) means its
    /// bank's; every other group as named.
    pub(super) fn own_group(base: u32, g: usize) -> usize {
        if base.wrapping_sub(SCRIPT_BASE) >> 20 >= 5 && (1..=2).contains(&g) { g + 4 } else { g }
    }

    /// The bank of a script at `base`: 1 for the team battle's groups 5 / 6 (engine addition: fighters 2 / 3, VRAM bank 1).
    pub(super) fn bank_of(base: u32) -> Bank {
        if (5..7).contains(&(base.wrapping_sub(SCRIPT_BASE) >> 20)) { Bank::B1 } else { Bank::B0 }
    }

    /// Register a script group: the blobs of `ScriptGroup[g][0..]` (None = empty slot).
    pub fn set_group(&mut self, g: usize, scripts: &[Option<Vec<u8>>]) {
        let mut v = Vec::new();
        for (n, s) in scripts.iter().enumerate() {
            match s {
                Some(b) if b.len() >= 0x1C && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0x1C => {
                    let base = SCRIPT_BASE + ((g as u32) << 20) + ((n as u32) << 14);
                    self.mem.map_static(base, Arc::from(b.as_slice()));
                    v.push(base);
                }
                _ => v.push(0),
            }
        }
        match g {
            0..5 => self.groups[g] = v,
            _ => self.groups_ext[g - 5] = v,
        }
    }

    /// The pool wipe of the fight-effect reset: all tasks and models gone, no destructors.
    pub fn clear(&mut self) {
        for h in std::mem::take(&mut self.order) {
            if let Some(Task { kind: Kind::Script { obj, owns_obj: true, .. }, .. }) = self.tasks.remove(&h) {
                self.mem.unmap(obj);
            }
        }
        self.tasks.clear();
        for (h, _) in std::mem::take(&mut self.models) {
            self.mem.unmap(h);
        }
        self.draws.clear();
    }

    /// Register a model group: the ED1C4 records of `EmitterGroup[g][0..]`.
    pub fn set_model_group(&mut self, g: usize, entries: &[Option<Vec<u8>>]) {
        self.model_groups[g] = entries.iter().map(|e| e.as_deref().and_then(crate::fight::fx::effect_models::Record::parse)).collect();
    }

    /// After deserialising: the static data from `fresh`, the VM the fight set up (`duel::setup::effect_vm`): the
    /// script blobs and EXE tables still unwritten, the model groups and the live models' records.
    /// Err when a static segment or a live model's record has no counterpart in `fresh`.
    pub fn rebind(&mut self, fresh: &Vm) -> Result<(), String> {
        self.mem.rebind(&fresh.mem)?;
        self.model_groups = fresh.model_groups.clone();
        self.groups_ext = fresh.groups_ext.clone();
        self.portable = fresh.portable;
        for (h, m) in self.models.iter_mut() {
            let (g, i) = m.rec;
            let Some(r) = self.model_groups.get(g).and_then(|v| v.get(i)).cloned().flatten() else {
                return Err(format!("fxvm: model {h:#010x}: no record EmitterGroup[{g}][{i}]"));
            };
            if !m.inst.fits(&r) {
                return Err(format!("fxvm: model {h:#010x} does not fit EmitterGroup[{g}][{i}]"));
            }
            m.inst.r = r;
        }
        if self.buf > 1 {
            return Err("fxvm: frame buffer".into());
        }
        Ok(())
    }

    pub fn live_models(&self) -> usize {
        self.models.len()
    }

    fn model(&mut self, h: u32) -> Option<&mut crate::fight::fx::effect_models::Instance> {
        self.models.iter_mut().find(|(k, _)| *k == h).map(|(_, m)| &mut m.inst)
    }

    /// model_free
    fn free_model(&mut self, h: u32) {
        if let Some(i) = self.models.iter().position(|(k, _)| *k == h) {
            self.models.remove(i);
            self.mem.unmap(h);
        }
    }

    /// The models' primitives of this frame (list 13): `project` maps a world point to (sx, sy, otz).
    pub fn model_prims(&self, project: &dyn Fn([i32; 3]) -> Option<(f32, f32, i32)>) -> Vec<crate::fight::fx::effect_models::ModelPrim> {
        let mut out = Vec::new();
        for (h, m) in self.models.iter().rev() {
            let ma = self.mem.u32(*h);
            let mr = [0, 1, 2].map(|r| [0, 1, 2].map(|c| self.mem.s16(ma + 2 * (3 * r + c)) as i32));
            let mt = [0, 1, 2].map(|k| self.mem.s32(ma + 0x14 + 4 * k));
            let first = out.len();
            m.inst.prims(self.buf, &mr, mt, project, &mut out);
            if Bank::of_vm_group(m.rec.0) == Bank::B1 {
                // the team battle's emitter groups 5 / 6 (engine addition): fighters 2 / 3's models draw from VRAM bank 1
                out[first..].iter_mut().for_each(|p| p.clut |= Bank::B1.clut_bit());
            }
        }
        out
    }

    // ------------------------------------------------------------------ native model API (C tasks)

    /// model_create from native code: `EmitterGroup[group][idx]`; returns the handle or 0.
    pub fn native_model(&mut self, group: usize, idx: usize) -> u32 {
        let Some(rec) = self.model_groups.get(group).and_then(|g| g.get(idx)).cloned().flatten() else { return 0 };
        let hm = self.alloc_obj(0x40);
        self.mem.w32(hm, hm + 0x20);
        for k in 0..3 {
            self.mem.w16(hm + 0x20 + 8 * k, 0x1000);
        }
        self.models.push((hm, ModelSlot { inst: crate::fight::fx::effect_models::Instance::new(rec, 0), slot: 0, rec: (group, idx) }));
        hm
    }

    pub fn model_alive(&self, h: u32) -> bool {
        h != 0 && self.models.iter().any(|(k, _)| *k == h)
    }

    /// Run `f` on a live model.
    pub fn with_model<R>(&mut self, h: u32, f: impl FnOnce(&mut crate::fight::fx::effect_models::Instance) -> R) -> Option<R> {
        self.model(h).map(f)
    }

    pub fn drop_model(&mut self, h: u32) {
        self.free_model(h);
    }

    /// Write the model MATRIX: 3×3 (4.12) and optionally the translation.
    pub fn set_model_matrix(&mut self, h: u32, r: &M3, t: Option<[i32; 3]>) {
        if !self.model_alive(h) {
            return;
        }
        let ma = self.mem.u32(h);
        self.write_m3(ma, r);
        if let Some(t) = t {
            for k in 0..3 {
                self.mem.w32(ma + 0x14 + 4 * k as u32, t[k] as u32);
            }
        }
    }

    pub fn model_translation(&self, h: u32) -> [i32; 3] {
        let ma = self.mem.u32(h);
        [0, 1, 2].map(|k| self.mem.s32(ma + 0x14 + 4 * k))
    }

    pub fn live(&self) -> usize {
        self.order.len()
    }

    fn handle_of(id: u32) -> u32 {
        HANDLE_BASE + id * 0x20
    }

    fn alloc_obj(&mut self, size: u32) -> u32 {
        let a = self.next_obj;
        self.next_obj = self.next_obj.wrapping_add((size + 0x1F) & !0xF);
        if self.next_obj >= HANDLE_BASE {
            self.next_obj = OBJ_BASE;
        }
        self.mem.map(a, vec![0; size as usize]);
        a
    }

    fn insert(&mut self, t: Task) -> u32 {
        let h = Self::handle_of(self.next_id);
        self.next_id += 1;
        self.tasks.insert(h, t);
        self.order.insert(0, h); // new tasks go to the head: they first run on the next frame
        h
    }

    /// spawn / spawn_owned (with `slot`): a new VM task for ScriptGroup[g][n].
    /// The object gets `O[0] = O`; `params` go to O+4..O+0x10 (the native parameter setters).
    pub fn spawn(&mut self, g: usize, n: usize, slot: Option<u32>, params: &[u32]) -> u32 {
        *self.stats.entry((g, n)).or_insert(0) += 1;
        let group = if g < 5 { self.groups.get(g) } else { self.groups_ext.get(g - 5) };
        let Some(&base) = group.and_then(|v| v.get(n)) else { return 0 };
        if base == 0 {
            log::debug!("fxvm: no script {g}/{n}");
            return 0;
        }
        let size = self.mem.u32(base + 4).clamp(0x20, 0x10000);
        let obj = self.alloc_obj(size);
        self.mem.w32(obj, obj);
        for (k, p) in params.iter().take(4).enumerate() {
            self.mem.w32(obj + 4 + 4 * k as u32, *p);
        }
        let pc = base + self.mem.u32(base);
        let (dtor, slot) = match slot {
            Some(s) => (Dtor::Owned, s),
            None => (Dtor::None, 0),
        };
        self.insert(Task { kind: Kind::Script { base, obj, pc, cond: 0, dp: 0, owns_obj: true }, slot, dtor, kill: false })
    }

    /// The object address of a task, 0 if there is none.
    pub fn obj_of(&self, h: u32) -> u32 {
        match self.tasks.get(&h).map(|t| &t.kind) {
            Some(Kind::Script { obj, .. }) => *obj,
            _ => 0,
        }
    }

    /// kill: deferred, the task is removed before its next update.
    pub fn kill(&mut self, h: u32) {
        if let Some(t) = self.tasks.get_mut(&h) {
            t.kill = true;
        }
    }

    pub fn alive(&self, h: u32) -> bool {
        self.tasks.get(&h).is_some_and(|t| !t.kill)
    }

    /// send_event: run the task once from its event-table entry `n`, then restore its pc.
    pub fn send_event(&mut self, h: u32, n: usize) {
        let Some(Kind::Script { base, pc, .. }) = self.tasks.get(&h).map(|t| t.kind.clone()) else { return };
        let tab = self.mem.u32(base + 0x18);
        if n != 0 && tab != 0 {
            let off = self.mem.s16(base + tab + 2 * n as u32) as i32;
            self.set_pc(h, (base as i32 + off) as u32);
        }
        self.budget = 200_000;
        self.run(h, 0);
        self.set_pc(h, pc);
    }

    fn set_pc(&mut self, h: u32, v: u32) {
        if let Some(Task { kind: Kind::Script { pc, .. }, .. }) = self.tasks.get_mut(&h) {
            *pc = v;
        }
    }

    /// One frame of list 4: kill pending tasks (destructor), run the others in list order.
    /// Tasks created during the walk wait for the next frame.
    pub fn tick(&mut self) {
        self.draws.clear();
        self.budget = 400_000;
        // lists 1 and 3: model instance checks and animation tracks run before the scripts (list 4)
        let buf = self.buf;
        let mut ended = Vec::new();
        for (h, m) in self.models.iter_mut() {
            if !m.inst.update(buf) {
                ended.push((*h, m.slot));
            }
        }
        for (h, slot) in ended {
            if slot != 0 && self.mem.u32(slot) == h {
                self.mem.w32(slot, 0);
            }
            self.free_model(h);
        }
        let snapshot = self.order.clone();
        for h in snapshot {
            let Some(t) = self.tasks.get(&h) else { continue };
            if t.kill {
                self.destroy(h);
                continue;
            }
            match t.kind {
                Kind::Script { .. } => {
                    self.run(h, 0);
                }
                Kind::Colour { .. } => self.colour_step(h),
                Kind::UvAnim { .. } => self.uv_anim_step(h),
            }
        }
    }

    /// Destructor and free.
    fn destroy(&mut self, h: u32) {
        let Some(t) = self.tasks.remove(&h) else { return };
        self.order.retain(|x| *x != h);
        match t.dtor {
            Dtor::Owned => {
                if t.slot != 0 {
                    self.mem.w32(t.slot, 0);
                }
                // spawn_owned's cleanup: run the cleanup entry hdr[4] once
                if let Kind::Script { base, .. } = t.kind {
                    let cleanup = self.mem.u32(base + 0x10);
                    if cleanup != 0 {
                        self.tasks.insert(h, Task { kind: t.kind.clone(), slot: 0, dtor: Dtor::None, kill: false });
                        self.set_pc(h, base + cleanup);
                        self.run(h, 0);
                        self.tasks.remove(&h);
                    }
                }
            }
            Dtor::Slot => {
                if t.slot != 0 {
                    self.mem.w32(t.slot, 0);
                }
            }
            Dtor::None => {}
        }
        if let Kind::Script { obj, owns_obj: true, .. } = t.kind {
            self.mem.unmap(obj);
        }
    }

    fn cond(&mut self, h: u32) -> &mut i32 {
        match &mut self.tasks.get_mut(&h).unwrap().kind {
            Kind::Script { cond, .. } => cond,
            _ => unreachable!(),
        }
    }

    fn get_cond(&self, h: u32) -> i32 {
        match self.tasks.get(&h).map(|t| &t.kind) {
            Some(Kind::Script { cond, .. }) => *cond,
            _ => 0,
        }
    }

    fn set_cond(&mut self, h: u32, v: i32) {
        if self.tasks.contains_key(&h) {
            *self.cond(h) = v;
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
