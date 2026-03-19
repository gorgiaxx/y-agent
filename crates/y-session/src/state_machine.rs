//! Session state machine — enforces valid state transitions.

use y_core::session::SessionState;

use crate::error::SessionManagerError;

/// Validates and enforces session state transitions.
///
/// State transition diagram:
/// ```text
///   Active → Paused
///   Active → Archived
///   Active → Merged
///   Active → Tombstone
///   Paused → Active
///   Paused → Archived
///   Paused → Tombstone
///   Archived → Tombstone
///   Merged → Tombstone
/// ```
pub struct StateMachine;

impl StateMachine {
    /// Check if a transition from `from` to `to` is valid.
    pub fn is_valid_transition(from: &SessionState, to: &SessionState) -> bool {
        matches!(
            (from, to),
            // Active → anything except itself
            (
                SessionState::Active,
                SessionState::Paused
                    | SessionState::Archived
                    | SessionState::Merged
                    | SessionState::Tombstone
            ) | (
                SessionState::Paused,
                SessionState::Active | SessionState::Archived | SessionState::Tombstone
            ) | (
                SessionState::Archived | SessionState::Merged,
                SessionState::Tombstone
            )
        )
    }

    /// Validate a state transition, returning an error if invalid.
    pub fn validate_transition(
        from: &SessionState,
        to: &SessionState,
    ) -> Result<(), SessionManagerError> {
        if from == to {
            return Err(SessionManagerError::InvalidTransition {
                from: format!("{from:?}"),
                to: format!("{to:?}"),
            });
        }

        if Self::is_valid_transition(from, to) {
            Ok(())
        } else {
            Err(SessionManagerError::InvalidTransition {
                from: format!("{from:?}"),
                to: format!("{to:?}"),
            })
        }
    }

    /// Get all valid target states from a given state.
    pub fn valid_transitions(from: &SessionState) -> Vec<SessionState> {
        let all = [
            SessionState::Active,
            SessionState::Paused,
            SessionState::Archived,
            SessionState::Merged,
            SessionState::Tombstone,
        ];

        all.into_iter()
            .filter(|to| Self::is_valid_transition(from, to))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_to_paused_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Active,
            &SessionState::Paused
        ));
    }

    #[test]
    fn test_active_to_archived_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Active,
            &SessionState::Archived
        ));
    }

    #[test]
    fn test_active_to_merged_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Active,
            &SessionState::Merged
        ));
    }

    #[test]
    fn test_active_to_tombstone_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Active,
            &SessionState::Tombstone
        ));
    }

    #[test]
    fn test_paused_to_active_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Paused,
            &SessionState::Active
        ));
    }

    #[test]
    fn test_archived_to_active_invalid() {
        assert!(!StateMachine::is_valid_transition(
            &SessionState::Archived,
            &SessionState::Active
        ));
    }

    #[test]
    fn test_tombstone_to_anything_invalid() {
        assert!(!StateMachine::is_valid_transition(
            &SessionState::Tombstone,
            &SessionState::Active
        ));
        assert!(!StateMachine::is_valid_transition(
            &SessionState::Tombstone,
            &SessionState::Paused
        ));
    }

    #[test]
    fn test_merged_to_tombstone_valid() {
        assert!(StateMachine::is_valid_transition(
            &SessionState::Merged,
            &SessionState::Tombstone
        ));
    }

    #[test]
    fn test_merged_to_active_invalid() {
        assert!(!StateMachine::is_valid_transition(
            &SessionState::Merged,
            &SessionState::Active
        ));
    }

    #[test]
    fn test_same_state_transition_invalid() {
        let result =
            StateMachine::validate_transition(&SessionState::Active, &SessionState::Active);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_transitions_from_active() {
        let targets = StateMachine::valid_transitions(&SessionState::Active);
        assert!(targets.contains(&SessionState::Paused));
        assert!(targets.contains(&SessionState::Archived));
        assert!(targets.contains(&SessionState::Merged));
        assert!(targets.contains(&SessionState::Tombstone));
        assert!(!targets.contains(&SessionState::Active));
    }

    #[test]
    fn test_valid_transitions_from_tombstone() {
        let targets = StateMachine::valid_transitions(&SessionState::Tombstone);
        assert!(targets.is_empty(), "tombstone is terminal");
    }

    #[test]
    fn test_validate_all_state_pairs_exhaustive() {
        // Ensure every pair is either valid or invalid — no panics.
        let states = [
            SessionState::Active,
            SessionState::Paused,
            SessionState::Archived,
            SessionState::Merged,
            SessionState::Tombstone,
        ];
        for from in &states {
            for to in &states {
                let _ = StateMachine::validate_transition(from, to);
            }
        }
    }
}
