//! Shared optical button material; re-exports `crate::lenses`.
pub(super) use crate::lenses::{ACTIVE, ENABLED, HOVER, LIGHT, Lenses, PRESSED, PRIMARY};
#[cfg(test)]
use crate::lenses::pixels;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glass_lens_has_directional_volume_and_correct_transparency() {
        let p = pixels(80, 2., 0x705040, ENABLED | PRIMARY);
        let at = |x: usize, y: usize, ch: usize| p[(y * 80 + x) * 4 + ch];
        assert_eq!(at(0, 0, 3), 0);
        assert!(
            at(13, 13, 0) > at(66, 66, 0) + 15,
            "light must be directional, not a flat ring"
        );
        assert!(at(40, 40, 3) > 35 && at(40, 40, 3) < 160);
    }
}
