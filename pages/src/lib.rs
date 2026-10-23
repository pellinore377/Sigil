//! Pages: attributed-text deltas, page ops with transformation, and the paragraph view.
pub mod delta;
pub mod ops;
pub mod page;
#[cfg(any(test, feature = "sim"))]
pub mod sim;
pub mod view;
pub use delta::{
    scalar_from_utf16, scalar_len, utf16_from_scalar, Attrs, Component, Delta, Insert, EMBED_CHAR,
    MAX_BODY, MAX_REPLAY,
};
pub use ops::{
    apply, apply_batch, apply_local, apply_local_batch, invert, invert_batch, transform,
    transform_batch, Action, Block, CellRef, Line, MetaOp, Op,
};
pub use page::{Kind, Meta, Page};
pub use view::{body_patch, body_view, paragraph_at, prints, view, BodyView, PageView};
