//! How much conversation history the client holds. The UI reports its viewport; this decides the rest.

/// Rows a single timeline scan returns.
pub const PAGE: usize = 64;
/// Messages kept decoded beyond the deepest visible one.
pub const AHEAD: usize = 96;
/// Distance from the loaded edge at which the client must ask again.
pub const MARGIN: usize = 48;
/// Screens kept composed past the viewport, towards older history, in tenths of a screen.
pub const CACHE_AHEAD_TENTHS: u32 = 30;
/// Screens kept composed behind the viewport, towards the newest message, in tenths of a screen.
pub const CACHE_BEHIND_TENTHS: u32 = 20;

pub struct Preload {
    /// Messages the client should hold.
    pub want: usize,
    /// Report a deeper viewport than this and the client must load again.
    pub reload_at: usize,
    /// Screens composed ahead of and behind the viewport, in tenths of a screen.
    pub cache_ahead_tenths: u32,
    pub cache_behind_tenths: u32,
}

/// `visible_end` is the number of messages between the newest one and the deepest the viewport has reached.
pub fn preload(visible_end: usize) -> Preload {
    let want = visible_end
        .saturating_add(AHEAD)
        .div_ceil(PAGE)
        .max(1)
        .saturating_mul(PAGE);
    Preload {
        want,
        reload_at: want.saturating_sub(MARGIN),
        cache_ahead_tenths: CACHE_AHEAD_TENTHS,
        cache_behind_tenths: CACHE_BEHIND_TENTHS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_viewport_already_buys_several_screens() {
        let first = preload(0);
        assert_eq!(first.want, 128);
        assert!(first.want >= AHEAD, "the buffer must outrun the viewport");
        assert_eq!(first.reload_at, 80);
    }

    #[test]
    fn asking_again_at_the_trigger_always_raises_the_target() {
        let mut want = preload(0).want;
        let mut at = preload(0).reload_at;
        for _ in 0..8 {
            assert!(at < want, "the trigger must fire before the loaded edge");
            let next = preload(at + 1);
            assert!(next.want > want, "a deeper viewport must want more");
            want = next.want;
            at = next.reload_at;
        }
    }

    #[test]
    fn targets_are_whole_pages_and_never_overflow() {
        for end in [0, 1, 63, 64, 65, 200, 10_000] {
            let value = preload(end);
            assert_eq!(value.want % PAGE, 0);
            assert!(value.want >= end + AHEAD);
            assert!(value.want - (end + AHEAD) < PAGE);
        }
        assert!(preload(usize::MAX).want > 0);
    }

    #[test]
    fn several_screens_are_kept_composed_in_both_directions() {
        let value = preload(0);
        assert!(value.cache_ahead_tenths >= 20 && value.cache_behind_tenths >= 20);
        assert!(value.cache_ahead_tenths >= value.cache_behind_tenths);
    }
}
