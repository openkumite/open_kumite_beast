//! The fight backdrop as the renderer takes it: two flat colours split at the horizon row.
//!
//! State: none
//!
//! The fight backdrop as the renderer takes it (`present/backdrop.rs` builds it): two flat colours behind the
//! stage model, split at the horizon row.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    pub top: [u8; 3],
    pub bottom: [u8; 3],
    /// frame-buffer rows 0..rows (of 480) are the top colour
    pub rows: i32,
}
