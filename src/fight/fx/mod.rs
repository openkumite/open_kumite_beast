//! The fight's effects (L2, ARCHITECTURE.md, Layers).
//!
//! The fight's effects (L2, ARCHITECTURE.md, Layers): the hit effects, the effect VM and its links, the native tasks,
//! the move effects and the 3-D effect models.

pub mod effect_models;
pub mod hit_effects;
pub mod host;
pub mod move_effects;
pub mod native_tasks;
pub mod script_link;
pub mod script_vm;

/// The effect VM's static block (`fxvm.static_block`): model_fx's UV animator, its UV deltas at +0 and its script at
/// +0x10; mapped into the VM at the table's origin.
pub(crate) const STATIC_BLOCK: &str = "fxvm.static_block";
/// The emitters' spawn timelines (`fxvm.timeline_records`, `fxvm.emitter_timelines`) and the four op 0x18 type B picks
/// from (`fxvm.timelines`: their addresses). An emitter keeps its timeline's address (the table's origin + offset).
pub(crate) const TIMELINE_RECORDS: &str = "fxvm.timeline_records";
pub(crate) const EMITTER_TIMELINES: &str = "fxvm.emitter_timelines";
pub(crate) const TIMELINES: &str = "fxvm.timelines";

/// The effect lists (pool 1) of a fight frame, after its hits: the effect scripts' links refreshed,
/// the native tasks' triggers and the stage pool, the effect scripts (list 4) before the effect lists 6..14, the
/// links written back, the native tasks, the hit effects. `frame`: the fight's frame counter (its parity picks the
/// VM's draw buffer).
pub fn effect_lists(h: &mut host::FxHost, data: &crate::fight::fight_data::Loaded, frame: u64) {
    script_link::refresh(h, data);
    h.vm.paused = false;
    h.vm.buf = (frame & 1) as usize;
    native_tasks::triggers(h, data);
    native_tasks::stage_pool(h, data);
    h.vm.tick();
    script_link::write_back(h);
    native_tasks::tick(h, data);
    h.fx.tick(&data.tables);
}
