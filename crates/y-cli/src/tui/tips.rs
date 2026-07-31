//! Rotating tips for the TUI, borrowed from kimi-code's toolbar tips.
//!
//! A hardcoded, weighted list of short usage hints. The weighted list is
//! expanded once into a smooth weighted round-robin (SWRR) rotation so
//! high-priority tips recur evenly. Rotation is stateless: the streaming
//! status bar derives the current tip from the animation tick counter, and
//! the welcome screen picks one pseudo-random tip per session.

use std::sync::LazyLock;

/// A single usage hint shown in the TUI.
pub struct Tip {
    /// Short English hint text (display width <= 60 columns).
    pub text: &'static str,
    /// Rotation weight: higher values recur more often per cycle. Minimum 1.
    pub weight: u8,
}

/// Ticks between tip changes in the streaming status bar (10 s at 100 ms).
pub const TIP_ROTATE_TICKS: u64 = 100;

/// Built-in tips. Every entry must reference a real y-agent feature (see
/// `tui/commands/registry.rs` and `tui/keys.rs`).
pub const TIPS: &[Tip] = &[
    Tip {
        text: "/goal <objective> runs a goal-directed task",
        weight: 2,
    },
    Tip {
        text: "! <cmd> runs a shell command",
        weight: 2,
    },
    Tip {
        text: "/ opens the command palette",
        weight: 2,
    },
    Tip {
        text: "Esc cancels a running response",
        weight: 1,
    },
    Tip {
        text: "/resume reopens a previous session",
        weight: 1,
    },
    Tip {
        text: "/compact compresses a long context",
        weight: 1,
    },
    Tip {
        text: "/tasks lists background tasks",
        weight: 1,
    },
    Tip {
        text: "/copy copies messages and tool output",
        weight: 1,
    },
    Tip {
        text: "/plan reviews a plan before executing",
        weight: 1,
    },
    Tip {
        text: "/loop iterates until the goal is done",
        weight: 1,
    },
    Tip {
        text: "/attach <path> adds a file to context",
        weight: 1,
    },
    Tip {
        text: "/shortcuts shows every keybinding",
        weight: 1,
    },
    Tip {
        text: "/mode switches fast, auto, plan, loop",
        weight: 1,
    },
    Tip {
        text: "/queue steers queued follow-ups mid-run",
        weight: 1,
    },
];

/// Expand tips into one rotation cycle via smooth weighted round-robin
/// (the nginx SWRR algorithm): the cycle length is the weight sum and each
/// tip recurs exactly `weight` times, spread evenly.
fn build_rotation(tips: &[Tip]) -> Vec<&'static str> {
    // (text, weight, current) per tip.
    let mut items: Vec<(&'static str, i64, i64)> = tips
        .iter()
        .map(|tip| (tip.text, i64::from(tip.weight.max(1)), 0i64))
        .collect();
    let total: i64 = items.iter().map(|(_, weight, _)| weight).sum();
    let mut sequence = Vec::with_capacity(total as usize);
    for _ in 0..total {
        let mut best = 0usize;
        for index in 0..items.len() {
            items[index].2 += items[index].1;
            if items[index].2 > items[best].2 {
                best = index;
            }
        }
        items[best].2 -= total;
        sequence.push(items[best].0);
    }
    sequence
}

/// The process-wide rotation cycle for [`TIPS`], built once on first use.
fn rotation() -> &'static [&'static str] {
    static ROTATION: LazyLock<Vec<&'static str>> = LazyLock::new(|| build_rotation(TIPS));
    &ROTATION
}

/// Tip for the streaming status bar at animation tick `tick_counter`.
///
/// Stateless rotation: the tip advances every [`TIP_ROTATE_TICKS`] ticks and
/// wraps around the cycle, so no timer or mutable state is needed.
pub fn tip_for_tick(tick_counter: u64) -> &'static str {
    let cycle = rotation();
    if cycle.is_empty() {
        return "";
    }
    cycle[(tick_counter / TIP_ROTATE_TICKS) as usize % cycle.len()]
}

/// Pick one tip from the cycle by seed (e.g. `SystemTime` nanos), used for
/// the per-session welcome-screen tip. Deterministic for a given seed.
pub fn random_tip(seed: u64) -> &'static str {
    let cycle = rotation();
    if cycle.is_empty() {
        return "";
    }
    cycle[seed as usize % cycle.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn test_rotation_length_equals_weight_sum() {
        let expected: usize = TIPS.iter().map(|tip| tip.weight.max(1) as usize).sum();
        assert_eq!(rotation().len(), expected);
    }

    #[test]
    fn test_rotation_frequency_matches_weight() {
        for tip in TIPS {
            let count = rotation().iter().filter(|text| **text == tip.text).count();
            assert_eq!(
                count,
                tip.weight.max(1) as usize,
                "tip {:?} must recur exactly weight times per cycle",
                tip.text
            );
        }
    }

    #[test]
    fn test_rotation_has_no_adjacent_duplicates() {
        let seq = rotation();
        for pair in seq.windows(2) {
            assert_ne!(pair[0], pair[1], "adjacent tips must differ");
        }
    }

    #[test]
    fn test_tip_for_tick_holds_then_advances() {
        let first = tip_for_tick(0);
        assert_eq!(tip_for_tick(TIP_ROTATE_TICKS - 1), first);
        assert_eq!(tip_for_tick(TIP_ROTATE_TICKS), rotation()[1]);
    }

    #[test]
    fn test_tip_for_tick_wraps_around() {
        let len = rotation().len() as u64;
        assert_eq!(tip_for_tick(len * TIP_ROTATE_TICKS), tip_for_tick(0));
    }

    #[test]
    fn test_random_tip_is_deterministic_per_seed() {
        assert_eq!(random_tip(42), random_tip(42));
        assert!(TIPS.iter().any(|tip| tip.text == random_tip(42)));
    }

    #[test]
    fn test_build_rotation_empty_input() {
        assert!(build_rotation(&[]).is_empty());
    }

    #[test]
    fn test_all_tips_fit_status_bar() {
        for tip in TIPS {
            assert!(
                UnicodeWidthStr::width(tip.text) <= 60,
                "tip too wide for the status bar: {:?}",
                tip.text
            );
        }
    }
}
