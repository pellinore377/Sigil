//! The composer's live preview: `red::text;` becomes a red "text" the moment
//! its `;` lands, and slides into the place the markup leaves.
//!
//! The input keeps the literal source — that is what is sent, and the engine
//! composes it — so this only draws. On every edit and every caret move the
//! source is mapped (`fx::compose_preview`) into cells that know where each
//! typed character sits now and where it settles, and the page interpolates
//! between the two. When the count of settled runs rises, an epoch ticks and
//! the page restarts its collapse; when nothing is settled, the page shows
//! the plain input and this costs nothing.

use crate::bridge::UiState;
use crate::AppWindow;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::Cell;

thread_local! {
    /// How many runs were settled at the last preview, so a rise is noticed.
    static SETTLED: Cell<usize> = const { Cell::new(0) };
    static EPOCH: Cell<i32> = const { Cell::new(0) };
}

/// `cursor` is the input's caret as a byte offset into `text`.
pub fn preview(ui: &mut UiState, win: &AppWindow, text: &str, cursor: usize) {
    let _ = ui;
    let base_px = win.global::<crate::Theme>().get_body() as f32;
    let max_w = (win.get_composer_w() as f32).max(base_px * 4.0);
    let cursor_char = text[..cursor.min(text.len())].chars().count();
    let p = crate::fx::compose_preview(text, cursor_char, base_px, max_w);

    let was = SETTLED.with(|s| s.replace(p.settled));
    if p.settled > was {
        EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
    }
    win.set_cp_epoch(EPOCH.with(|e| e.get()));
    win.set_cp_styled(p.settled > 0);
    win.set_cp_caret_x(p.caret.0);
    win.set_cp_caret_y(p.caret.1);
    win.set_cp_caret_fx(p.caret_from.0);
    win.set_cp_caret_fy(p.caret_from.1);
    if p.settled == 0 {
        // The plain input is showing; no cells to keep.
        win.set_cp_cells(ModelRc::new(VecModel::from(Vec::<crate::ComposerCell>::new())));
        return;
    }
    let rows: Vec<crate::ComposerCell> = p
        .cells
        .into_iter()
        .map(|c| {
            let parsed = c.color.as_deref().and_then(crate::rows::hex_color);
            crate::ComposerCell {
                ch: c.ch.into(),
                from_x: c.from.0,
                from_y: c.from.1,
                to_x: c.to.0,
                to_y: c.to.1,
                size: c.size,
                has_color: parsed.is_some(),
                color: parsed.unwrap_or_default(),
                bold: c.bold,
                italic: c.italic,
                mono: c.mono,
                gone: c.gone,
            }
        })
        .collect();
    win.set_cp_cells(ModelRc::new(VecModel::from(rows)));
}

/// The composer was cleared (a send, a room change): nothing is settled.
pub fn reset(win: &AppWindow) {
    SETTLED.with(|s| s.set(0));
    win.set_cp_styled(false);
    win.set_cp_cells(ModelRc::new(VecModel::from(Vec::<crate::ComposerCell>::new())));
}
