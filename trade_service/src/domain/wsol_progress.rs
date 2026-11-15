use super::reconciliation::{ChainCommitment, ChainObservation};
use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub(crate) struct WsolSignatureObservation {
    pub chain: ChainObservation,
    pub context_slot: u64,
}

impl WsolSignatureObservation {
    pub(crate) fn outcome_for(&self, signature: &str) -> Result<WsolObservedOutcome> {
        if signature.is_empty() || self.chain.signature != signature {
            bail!("WSOL observation does not match the original signature");
        }
        if self.context_slot == 0 || self.chain.slot == 0 || self.chain.slot > self.context_slot {
            bail!("WSOL observation has an invalid transaction or context slot");
        }
        // WSOL has no persisted intermediate commitment rank: only rooted evidence is terminal.
        Ok(match (self.chain.commitment, self.chain.error.is_some()) {
            (ChainCommitment::Finalized, true) => WsolObservedOutcome::Failed,
            (ChainCommitment::Finalized, false) => WsolObservedOutcome::Confirmed,
            _ => WsolObservedOutcome::Submitted,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolState {
    Reserved,
    Signed,
    Unknown,
    Submitted,
    Confirmed,
    Failed,
}
impl WsolState {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "reserved" => Self::Reserved,
            "signed" => Self::Signed,
            "unknown" => Self::Unknown,
            "submitted" => Self::Submitted,
            "confirmed" => Self::Confirmed,
            "failed" => Self::Failed,
            _ => bail!("invalid persisted WSOL submission state"),
        })
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Signed => "signed",
            Self::Unknown => "unknown",
            Self::Submitted => "submitted",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum WsolBroadcastOutcome {
    Submitted,
    Unknown,
}
impl WsolBroadcastOutcome {
    pub(crate) fn state(self) -> WsolState {
        match self {
            Self::Submitted => WsolState::Submitted,
            Self::Unknown => WsolState::Unknown,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsolObservedOutcome {
    Submitted,
    Confirmed,
    Failed,
}
impl WsolObservedOutcome {
    pub(crate) fn state(self) -> WsolState {
        match self {
            Self::Submitted => WsolState::Submitted,
            Self::Confirmed => WsolState::Confirmed,
            Self::Failed => WsolState::Failed,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum WsolUpdate {
    Broadcast(WsolBroadcastOutcome),
    Observation(WsolObservedOutcome),
    SimulationUncertain,
}
impl WsolUpdate {
    pub(crate) fn target(self) -> WsolState {
        match self {
            Self::Broadcast(outcome) => outcome.state(),
            Self::Observation(outcome) => outcome.state(),
            Self::SimulationUncertain => WsolState::Unknown,
        }
    }
}

/// False means a duplicate or weaker late result; it must not rewrite evidence or timestamps.
pub(crate) fn should_advance(current: WsolState, update: WsolUpdate) -> Result<bool> {
    use WsolState::*;
    let next = update.target();
    match (current, next) {
        (Reserved, _) => bail!("WSOL outcome requires persisted signed evidence"),
        (Confirmed, Failed) | (Failed, Confirmed)
            if matches!(update, WsolUpdate::Observation(_)) =>
        {
            bail!("conflicting terminal WSOL observations; retain current record")
        }
        (Confirmed | Failed, _) => Ok(false),
        (Submitted, Unknown | Submitted) => Ok(false),
        (Unknown, Unknown) => Ok(false),
        _ => Ok(true),
    }
}

/// A read may observe another request's progress, but never a rollback or a different terminal outcome.
pub(crate) fn validate_read_progress(before: WsolState, after: WsolState) -> Result<()> {
    use WsolState::*;
    let allowed = before == after
        || match before {
            Reserved => true,
            Signed => matches!(after, Unknown | Submitted | Confirmed | Failed),
            Unknown => matches!(after, Submitted | Confirmed | Failed),
            Submitted => matches!(after, Confirmed | Failed),
            Confirmed | Failed => false,
        };
    if !allowed {
        bail!("WSOL status readback regressed or changed a terminal outcome");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsol_status_boundary_snapshot_matrix_allows_only_forward_reads_or_the_same_terminal() {
        use WsolState::*;
        let states = [Reserved, Signed, Unknown, Submitted, Confirmed, Failed];
        let expected = [
            [true, true, true, true, true, true],
            [false, true, true, true, true, true],
            [false, false, true, true, true, true],
            [false, false, false, true, true, true],
            [false, false, false, false, true, false],
            [false, false, false, false, false, true],
        ];
        for (row, before) in states.into_iter().enumerate() {
            for (column, after) in states.into_iter().enumerate() {
                assert_eq!(
                    validate_read_progress(before, after).is_ok(),
                    expected[row][column],
                    "{before:?} -> {after:?}"
                );
            }
        }
    }

    fn observation(commitment: ChainCommitment, error: Option<&str>) -> WsolSignatureObservation {
        WsolSignatureObservation {
            chain: ChainObservation {
                signature: "original-signature".into(),
                commitment,
                slot: 42,
                error: error.map(Into::into),
            },
            context_slot: 43,
        }
    }

    #[test]
    fn wsol_observations_require_finalized_evidence_for_terminal_success_or_failure() {
        for commitment in [
            ChainCommitment::Processed,
            ChainCommitment::Confirmed,
            ChainCommitment::Finalized,
        ] {
            for error in [None, Some("synthetic chain error")] {
                assert_eq!(
                    observation(commitment, error)
                        .outcome_for("original-signature")
                        .unwrap(),
                    if commitment != ChainCommitment::Finalized {
                        WsolObservedOutcome::Submitted
                    } else if error.is_some() {
                        WsolObservedOutcome::Failed
                    } else {
                        WsolObservedOutcome::Confirmed
                    }
                );
            }
        }
    }

    #[test]
    fn wsol_observations_reject_wrong_signature_zero_and_future_slots() {
        let original = observation(ChainCommitment::Finalized, None);
        assert!(original.outcome_for("another-signature").is_err());
        assert!(original.outcome_for("").is_err());
        for (slot, context_slot) in [(0, 43), (42, 0), (44, 43)] {
            let mut wrong = original.clone();
            wrong.chain.slot = slot;
            wrong.context_slot = context_slot;
            assert!(wrong.outcome_for("original-signature").is_err());
        }
        let mut wrong = original;
        wrong.chain.signature.clear();
        assert!(wrong.outcome_for("").is_err());
    }

    #[test]
    fn wsol_progress_matrix_never_downgrades_or_reopens_terminal_states() {
        use WsolState::*;
        let updates = [
            WsolUpdate::SimulationUncertain,
            WsolUpdate::Broadcast(WsolBroadcastOutcome::Unknown),
            WsolUpdate::Broadcast(WsolBroadcastOutcome::Submitted),
            WsolUpdate::Observation(WsolObservedOutcome::Submitted),
            WsolUpdate::Observation(WsolObservedOutcome::Confirmed),
            WsolUpdate::Observation(WsolObservedOutcome::Failed),
        ];
        for (state, expected) in [
            (Reserved, [-1, -1, -1, -1, -1, -1]),
            (Signed, [1, 1, 1, 1, 1, 1]),
            (Unknown, [0, 0, 1, 1, 1, 1]),
            (Submitted, [0, 0, 0, 0, 1, 1]),
            (Confirmed, [0, 0, 0, 0, 0, -1]),
            (Failed, [0, 0, 0, 0, -1, 0]),
        ] {
            assert_eq!(WsolState::parse(state.as_str()).unwrap(), state);
            for (update, expected) in updates.into_iter().zip(expected) {
                let result = should_advance(state, update);
                assert_eq!(
                    result.map(|advance| i32::from(advance)).unwrap_or(-1),
                    expected,
                    "{state:?} {update:?}"
                );
            }
        }
        assert!(WsolState::parse("corrupt").is_err());
    }
}
