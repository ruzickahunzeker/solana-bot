use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone)]
pub struct RiskDecision {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Clone)]
pub struct RiskEngine {
    paused: Arc<AtomicBool>,
}

impl RiskEngine {
    pub fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn check_live_trade_allowed(&self, dry_run: bool) -> RiskDecision {
        if dry_run {
            return RiskDecision {
                allowed: true,
                reason: "dry-run".to_string(),
            };
        }

        if self.is_paused() {
            return RiskDecision {
                allowed: false,
                reason: "global panic pause is active".to_string(),
            };
        }

        RiskDecision {
            allowed: true,
            reason: "allowed".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_mode_does_not_require_a_second_global_switch() {
        assert!(RiskEngine::new().check_live_trade_allowed(false).allowed);
    }

    #[test]
    fn panic_pause_blocks_live_but_not_dry_run() {
        let risk = RiskEngine::new();
        risk.set_paused(true);
        assert!(!risk.check_live_trade_allowed(false).allowed);
        assert!(risk.check_live_trade_allowed(true).allowed);
    }
}
