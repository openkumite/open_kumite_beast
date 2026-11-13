//! The headless playback (`simulate`): a replay's steps run as the app plays them.
//!
//! Engine addition.
//!
//! The headless playback (`simulate`: the `replay` binary, the tests; `simulate_async`: the wasm `replay --trace`):
//! the steps run as the app plays them, the static data loaded as they ask for it. `Headless` is that playback a
//! step at a time with each step's output (`replay --video` draws and mixes it).

use super::{Player, Replay};
use crate::frontend::LoadSpec;
use crate::session::{Entry, FrameInput, GameState, hash};

/// A headless playback's result.
#[derive(Clone, Copy, Debug)]
pub struct Summary {
    pub frames: u32,
    pub checks: u32,
    pub strict: bool,
    /// the last state's hash
    pub hash: u64,
}

/// The replay re-simulated without the app, as the app plays it: the static data loaded as the steps ask for it
/// (a match when the recorded input says it was loaded, files when they were fetched), every checkpoint compared;
/// `each` sees the state after every step. Err at the first mismatch, or for a file whose steps cannot run.
#[cfg(not(target_arch = "wasm32"))]
pub fn simulate(assets: &crate::assets::Assets, replay: Replay, each: impl FnMut(&GameState)) -> Result<Summary, String> {
    pollster::block_on(simulate_async(assets, replay, each))
}

/// `simulate` on any target (the browser's data loads are async: the wasm `replay --trace` entry).
pub async fn simulate_async(assets: &crate::assets::Assets, replay: Replay, mut each: impl FnMut(&GameState)) -> Result<Summary, String> {
    let mut h = Headless::new(assets, replay).await?;
    while h.step().await? {
        each(&h.s);
    }
    Ok(h.summary())
}

/// `simulate`'s playback, a step at a time.
pub struct Headless<'a> {
    assets: &'a crate::assets::Assets,
    store: crate::session::AssetStore,
    pub(super) s: GameState,
    p: Player,
    /// the match and the files asked for, not loaded yet
    load: Option<LoadSpec>,
    fetch: Option<Vec<String>>,
    /// the steps' `Out` keeps the OKB_SOUNDLOG frame marks (`Out::new`)
    marks: bool,
}

/// A `Headless` playback's seek snapshot (as the app's: the state, the data the next step reads, what was asked
/// for, the file's position).
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) struct HeadlessKey {
    pub(super) snap: Vec<u8>,
    pub(super) held: crate::session::rollback::Held,
    load: Option<LoadSpec>,
    fetch: Option<Vec<String>>,
    pos: super::Pos,
}

impl<'a> Headless<'a> {
    pub(super) async fn new(assets: &'a crate::assets::Assets, replay: Replay) -> Result<Self, String> {
        Ok(Self::open(assets, replay, false).await?.0)
    }

    /// The playback at its start and the start's output (`session::new_state`'s); `marks`: every `Out` keeps the
    /// OKB_SOUNDLOG frame marks.
    pub async fn open(assets: &'a crate::assets::Assets, replay: Replay, marks: bool) -> Result<(Self, crate::session::Out), String> {
        use std::sync::Arc;
        let start = replay.start();
        super::br1_fits(&start, crate::assets::manifest::br1_install_id(assets))?;
        if let Some(w) = super::content_warning(&start, crate::assets::manifest::install_id(assets).await.ok()) {
            log::warn!("replay: {w}");
        }
        let mut store = crate::session::AssetStore::default();
        match &start.entry {
            Entry::DirectFight { p1, p2, stage } => {
                let data =
                    crate::fight::fight_data::load_match(assets, [p1.as_str(), p2.as_str()], replay.header.costume, stage, false, replay.sim_version).await?;
                store.fight = Some(Arc::new(data));
            }
            _ => store.front = Some(Arc::new(crate::frontend::load(assets).await?)),
        }
        let p = Player::new(replay);
        let mut out = crate::session::Out::new(marks);
        let s = crate::session::new_state(&start, &store.assets(p.prefs()), &mut out);
        Ok((Self { assets, store, s, p, load: None, fetch: None, marks }, out))
    }

    /// The next step and its checkpoint; false at the end.
    pub(super) async fn step(&mut self) -> Result<bool, String> {
        Ok(self.step_out().await?.is_some())
    }

    /// The next step and its checkpoint, with the step's output (its requests served as far as the logic's data
    /// goes: a load, a fetch, a fight's end); None at the end.
    pub async fn step_out(&mut self) -> Result<Option<crate::session::Out>, String> {
        use crate::session::{Out, Request, step};
        use std::sync::Arc;
        let Some((i, cheats, prefs)) = self.p.next_step() else { return Ok(None) };
        let (assets, store, s) = (self.assets, &mut self.store, &mut self.s);
        if i.flags & FrameInput::LOADED != 0 {
            let load = self.load.take().ok_or_else(|| format!("frame {}: a load completes that was not asked for", s.frame))?;
            let data = match load {
                LoadSpec::Match(spec) => {
                    let chr = spec.chr.map(crate::assets::roster::code);
                    let bgm = s.front.as_ref().is_some_and(|f| f.arcade_bgm());
                    crate::fight::fight_data::load_match(assets, chr, spec.costume, crate::fight::fight_data::stage_code(spec.stage), bgm, s.sim_version)
                        .await?
                }
                // the team battle (engine addition)
                LoadSpec::Team(spec) => {
                    let chr = spec.chr.map(crate::assets::roster::code);
                    crate::fight::fight_data::load_team(assets, chr, spec.costume, crate::fight::fight_data::stage_code(spec.stage), s.sim_version).await?
                }
            };
            store.fight = Some(Arc::new(data));
        }
        if i.flags & FrameInput::FETCHED != 0 {
            let names: Vec<String> = self.fetch.take().ok_or_else(|| format!("frame {}: a fetch completes that was not asked for", s.frame))?;
            for n in names {
                // a movie's music: its `.xa` first (the video export plays it); an imported ending's from that install
                if let Ok(b) = assets.fetch(&n).await {
                    store.files.insert(n, b.into());
                }
            }
        }
        let mut out = Out::new(self.marks);
        step(s, &store.assets(prefs), i, &cheats, &mut out);
        for r in &out.requests {
            match r {
                Request::Load(spec) => {
                    store.fight = None;
                    self.load = Some(LoadSpec::Match(*spec));
                }
                Request::LoadTeam(spec) => {
                    store.fight = None;
                    self.load = Some(LoadSpec::Team(*spec));
                }
                Request::TeamOver { .. } => store.fight = None,
                Request::Fetch(names) => {
                    for n in names {
                        store.files.remove(n);
                    }
                    self.fetch = Some(names.clone());
                }
                Request::FightOver { backdrop, .. } => {
                    let data = store.fight.take();
                    if *backdrop {
                        store.backdrop = data;
                    }
                }
                _ => {}
            }
        }
        if s.backdrop.is_none() {
            store.backdrop = None;
        }
        store.prune(s);
        self.p.check(s)?;
        Ok(Some(out))
    }

    /// The state after the last step.
    pub fn state(&self) -> &GameState {
        &self.s
    }

    /// The static data the next step reads (the last step's requests served).
    pub fn store(&self) -> &crate::session::AssetStore {
        &self.store
    }

    pub(super) fn summary(&self) -> Summary {
        Summary { frames: self.p.frame(), checks: self.p.checked(), strict: self.p.strict(), hash: hash(&self.s) }
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn key(&self) -> HeadlessKey {
        let held = crate::session::rollback::Held::of(&self.store, self.p.prefs());
        HeadlessKey { snap: crate::session::save_state(&self.s), held, load: self.load, fetch: self.fetch.clone(), pos: self.p.pos() }
    }

    /// Back (or on) to a snapshot, as the app's seek: the state loaded with its data, the file's position.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(super) fn seek(&mut self, k: &HeadlessKey) -> Result<(), String> {
        self.s = crate::session::load_state(&k.snap, &k.held.assets())?;
        self.store = k.held.clone().into_store();
        (self.load, self.fetch) = (k.load, k.fetch.clone());
        self.p.seek(k.pos);
        Ok(())
    }
}
