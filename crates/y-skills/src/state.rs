//! Skill state machine: enforces valid lifecycle transitions.
//!
//! Design reference: skills-knowledge-design.md state diagram.
//!
//! ```text
//! [*] → Submitted → Analyzing → Classified → Rejected → [*]
//!                                           → Transforming → Transformed → Registered → Active
//!                                                                                     → Deprecated → [*]
//! ```

// Re-export the enum from y-core (canonical definition).
pub use y_core::skill::SkillState;

/// Error returned when an invalid state transition is attempted.
#[derive(Debug, thiserror::Error)]
#[error("invalid state transition: {from} → {to}")]
pub struct InvalidTransition {
    pub from: SkillState,
    pub to: SkillState,
}

/// Enforces valid skill lifecycle transitions.
#[derive(Debug)]
pub struct SkillStateMachine {
    state: SkillState,
}

impl SkillStateMachine {
    /// Create a new state machine starting at the given state.
    pub fn new(initial: SkillState) -> Self {
        Self { state: initial }
    }

    /// Create a new state machine starting at `Submitted`.
    pub fn submitted() -> Self {
        Self::new(SkillState::Submitted)
    }

    /// Create a new state machine starting at `Registered` (for manually-created skills).
    pub fn registered() -> Self {
        Self::new(SkillState::Registered)
    }

    /// Current state.
    pub fn state(&self) -> SkillState {
        self.state
    }

    /// Attempt a state transition. Returns error if the transition is not allowed.
    pub fn transition(&mut self, to: SkillState) -> Result<(), InvalidTransition> {
        if Self::is_valid_transition(self.state, to) {
            self.state = to;
            Ok(())
        } else {
            Err(InvalidTransition {
                from: self.state,
                to,
            })
        }
    }

    /// Check if a transition from `from` to `to` is valid per the design diagram.
    pub fn is_valid_transition(from: SkillState, to: SkillState) -> bool {
        matches!(
            (from, to),
            (SkillState::Submitted, SkillState::Analyzing)
                | (SkillState::Analyzing, SkillState::Classified)
                | (
                    SkillState::Classified,
                    SkillState::Rejected | SkillState::Transforming
                )
                | (SkillState::Transforming, SkillState::Transformed)
                | (
                    SkillState::Transformed | SkillState::Active,
                    SkillState::Registered
                )
                | (
                    SkillState::Registered,
                    SkillState::Active | SkillState::Deprecated
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-SK-S2-05: State machine rejects invalid transitions.
    #[test]
    fn test_state_machine_rejects_invalid_transitions() {
        let mut sm = SkillStateMachine::submitted();

        // Cannot jump from Submitted directly to Registered
        assert!(sm.transition(SkillState::Registered).is_err());
        assert_eq!(sm.state(), SkillState::Submitted);

        // Cannot jump from Submitted to Active
        assert!(sm.transition(SkillState::Active).is_err());

        // Cannot jump from Submitted to Deprecated
        assert!(sm.transition(SkillState::Deprecated).is_err());

        // Cannot jump from Submitted to Transformed
        assert!(sm.transition(SkillState::Transformed).is_err());
    }

    /// T-SK-S2-06: State machine allows full lifecycle Submitted → Active.
    #[test]
    fn test_state_machine_full_lifecycle() {
        let mut sm = SkillStateMachine::submitted();

        sm.transition(SkillState::Analyzing).unwrap();
        assert_eq!(sm.state(), SkillState::Analyzing);

        sm.transition(SkillState::Classified).unwrap();
        assert_eq!(sm.state(), SkillState::Classified);

        sm.transition(SkillState::Transforming).unwrap();
        assert_eq!(sm.state(), SkillState::Transforming);

        sm.transition(SkillState::Transformed).unwrap();
        assert_eq!(sm.state(), SkillState::Transformed);

        sm.transition(SkillState::Registered).unwrap();
        assert_eq!(sm.state(), SkillState::Registered);

        sm.transition(SkillState::Active).unwrap();
        assert_eq!(sm.state(), SkillState::Active);
    }

    /// Active → Registered (deselected) is valid.
    #[test]
    fn test_state_machine_active_to_registered() {
        let mut sm = SkillStateMachine::registered();
        sm.transition(SkillState::Active).unwrap();
        sm.transition(SkillState::Registered).unwrap();
        assert_eq!(sm.state(), SkillState::Registered);
    }

    /// Registered → Deprecated is valid.
    #[test]
    fn test_state_machine_registered_to_deprecated() {
        let mut sm = SkillStateMachine::registered();
        sm.transition(SkillState::Deprecated).unwrap();
        assert_eq!(sm.state(), SkillState::Deprecated);
    }

    /// Classified → Rejected is valid.
    #[test]
    fn test_state_machine_classified_to_rejected() {
        let mut sm = SkillStateMachine::submitted();
        sm.transition(SkillState::Analyzing).unwrap();
        sm.transition(SkillState::Classified).unwrap();
        sm.transition(SkillState::Rejected).unwrap();
        assert_eq!(sm.state(), SkillState::Rejected);
    }

    /// Display formatting works.
    #[test]
    fn test_state_display() {
        assert_eq!(SkillState::Submitted.to_string(), "submitted");
        assert_eq!(SkillState::Active.to_string(), "active");
        assert_eq!(SkillState::Deprecated.to_string(), "deprecated");
    }
}
