//! Structured operator/caller error representation (`SHIP_GATE.md` `:26`) with built-in secret
//! scrubbing before text or JSON emission, re-exported from the `league-error` leaf crate.

pub use league_error::{StructuredError, ToStructuredError, scrub_secret_text};

use crate::replay::ReplayLayoutSeedError;
use crate::types::EngineError;

impl ToStructuredError for EngineError {
    fn to_structured_error(&self) -> StructuredError {
        let hint = match self {
            Self::InvalidConfig(_) => {
                "check TableSpec, mode, seed pool, and replay fields against table validation rules"
            }
            Self::InvalidPhase(_) => {
                "submit the operation during the matching TurnPhase (TalkAndMarket, Commit, Reveal, or AwaitingMandateReveal)"
            }
            Self::InvalidSeat(_) => "use a valid zero-based seat index in 0..seat_count",
            Self::InvalidOrder(_) => {
                "adjust quantities, deadlines, assets, or VRF/replay parameters to satisfy rule bounds"
            }
            Self::UnknownObject(_) => {
                "verify the offer, check, escrow, project, or cartel domain ID exists in current state"
            }
            Self::Unauthorized(_) => {
                "ensure the acting seat owns the object, holds the required charter, and is not wash-frozen"
            }
            Self::PerishableEscrowForbidden => {
                "lock CRED collateral rather than perishable raw commodities inside TokenEscrow"
            }
            Self::InsufficientBalance(_) => {
                "reduce the order size or acquire enough free CRED or commodity units first"
            }
            Self::AlreadySubmitted(_) => {
                "wait for the next turn or phase rather than submitting a duplicate commitment or reveal"
            }
            Self::CommitmentMismatch(_) => {
                "reveal the exact SealedOrderBundle or Mandate and RevealSalt committed earlier"
            }
            Self::DomainAccessDenied(_) => {
                "obtain cartel domain membership or an active ClearinghouseBroker charter before accessing this domain"
            }
            Self::CancelLocked => {
                "cancel resting offers before Phase-1 cancel lock engages; late cancellations are rejected"
            }
            Self::SeasonEnded(_) => {
                "verify the replay log contains a complete closed season from Turn 1 through AwaitingMandateReveal"
            }
            Self::UnsupportedReplayVersion { .. } => {
                "upgrade the reader to support this replay format_version before loading the file"
            }
        };
        StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

impl ToStructuredError for ReplayLayoutSeedError {
    fn to_structured_error(&self) -> StructuredError {
        let hint = match self {
            Self::SealedSeedInClear { .. } => {
                "redact sealed-range layout seeds via SeasonReplayLog::new_sealed or from_rig_seed"
            }
            Self::InvalidSealedPool { .. } => {
                "set sealed layout seed pool to 'sealed' with its 64-hex SHA-256 digest"
            }
            Self::InvalidSealedDigest => {
                "provide a 64-character lowercase ASCII hex SHA-256 digest for the sealed layout seed"
            }
            Self::NonSealedSeedInSealed { .. } => {
                "use ReplayLayoutSeed::try_clear or SeasonReplayLog::from_rig_seed for non-sealed seeds (< 2^63)"
            }
        };
        StructuredError::new(self.code(), self.to_string(), hint, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn reexported_structured_error_matches_7fd3b481_pre_change_json_and_format_text() {
        assert_eq!(
            TypeId::of::<StructuredError>(),
            TypeId::of::<league_error::StructuredError>()
        );
        assert_eq!(
            TypeId::of::<crate::StructuredError>(),
            TypeId::of::<league_error::StructuredError>()
        );

        let c1 = crate::StructuredError::new(
            "INVALID_CONFIG",
            "bad config OPENROUTER_API_KEY=sk-or-secret123",
            "check TableSpec",
            false,
        );
        assert_eq!(
            c1.to_json_string()
                .unwrap_or_else(|e| panic!("c1 json failed: {e}")),
            "{\"code\":\"INVALID_CONFIG\",\"message\":\"bad config OPENROUTER_API_KEY=[REDACTED]\",\"hint\":\"check TableSpec\",\"retryable\":false}"
        );
        assert_eq!(
            c1.format_text(),
            "[INVALID_CONFIG] bad config OPENROUTER_API_KEY=[REDACTED]; hint: check TableSpec"
        );

        let c2 = crate::StructuredError::new(
            "TURN_LOCKED",
            "lock busy sk-or-v1-abc123",
            "wait and retry",
            true,
        );
        assert_eq!(
            c2.to_json_string()
                .unwrap_or_else(|e| panic!("c2 json failed: {e}")),
            "{\"code\":\"TURN_LOCKED\",\"message\":\"lock busy [REDACTED]\",\"hint\":\"wait and retry\",\"retryable\":true}"
        );
        assert_eq!(
            c2.format_text(),
            "[TURN_LOCKED] lock busy [REDACTED]; hint: wait and retry [retryable]"
        );

        let c3 = crate::StructuredError::new("RELAY_IO", "io error", "check path", false)
            .with_cause("GEMINI_API_KEY=AIzaSySecret12345 failed");
        assert_eq!(
            c3.to_json_string()
                .unwrap_or_else(|e| panic!("c3 json failed: {e}")),
            "{\"code\":\"RELAY_IO\",\"message\":\"io error\",\"hint\":\"check path\",\"cause\":\"GEMINI_API_KEY=[REDACTED] failed\",\"retryable\":false}"
        );
        assert_eq!(
            c3.format_text(),
            "[RELAY_IO] io error (cause: GEMINI_API_KEY=[REDACTED] failed); hint: check path"
        );

        let c4 = crate::StructuredError::new(
            "SIGN_ERR",
            "failed signing with sEd7f3A9kL2mN4pQ6rT8vW1xY3zB5cD",
            "rotate seed",
            true,
        )
        .with_cause("GEMINI_API_KEY=AIzaSySecret12345 caused retryable failure");
        assert_eq!(
            c4.to_json_string()
                .unwrap_or_else(|e| panic!("c4 json failed: {e}")),
            "{\"code\":\"SIGN_ERR\",\"message\":\"failed signing with [REDACTED]\",\"hint\":\"rotate seed\",\"cause\":\"GEMINI_API_KEY=[REDACTED] caused retryable failure\",\"retryable\":true}"
        );
        assert_eq!(
            c4.format_text(),
            "[SIGN_ERR] failed signing with [REDACTED] (cause: GEMINI_API_KEY=[REDACTED] caused retryable failure); hint: rotate seed [retryable]"
        );

        let eng_err = EngineError::InvalidOrder(
            "Bearer\ttab_secret_tok OPENROUTER_API_KEY=sk-or-secret999".to_owned(),
        );
        let eng_se = eng_err.to_structured_error();
        assert_eq!(eng_se.code, "INVALID_ORDER");
        assert!(!eng_se.message.contains("tab_secret_tok"));
        assert!(!eng_se.message.contains("sk-or-secret999"));
        assert!(eng_se.message.contains("Bearer\t[REDACTED]"));

        let seed_err = ReplayLayoutSeedError::InvalidSealedDigest;
        assert_eq!(
            seed_err.to_structured_error().code,
            "INVALID_SEALED_SEED_DIGEST"
        );
        assert_eq!(
            crate::scrub_secret_text("Bearer\ttok_abc"),
            "Bearer\t[REDACTED]"
        );
    }
}
