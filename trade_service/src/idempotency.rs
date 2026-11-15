pub use crate::application::ports::trade_admission::MemoryReservation as ReserveResult;
use dashmap::DashMap;

#[derive(Default)]
pub struct IdempotencyStore {
    keys: DashMap<(String, String), Reservation>,
}

struct Reservation {
    request_id: String,
    request_hash: String,
    aggregator_sol: bool,
    execution_owner: Option<String>,
}

impl IdempotencyStore {
    pub fn reserve(
        &self,
        wallet_id: &str,
        key: &str,
        request_id: &str,
        request_hash: &str,
        aggregator_sol: bool,
    ) -> ReserveResult {
        match self.keys.entry((wallet_id.to_string(), key.to_string())) {
            dashmap::mapref::entry::Entry::Occupied(existing) => {
                let existing = existing.get();
                if existing.request_hash == request_hash
                    && existing.aggregator_sol == aggregator_sol
                {
                    ReserveResult::Duplicate(existing.request_id.clone())
                } else {
                    ReserveResult::Conflict
                }
            }
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(Reservation {
                    request_id: request_id.to_string(),
                    request_hash: request_hash.to_string(),
                    aggregator_sol,
                    execution_owner: None,
                });
                ReserveResult::Reserved
            }
        }
    }

    pub(crate) fn claim_execution(
        &self,
        execution: &crate::application::trade_admission::AdmittedTrade,
        parent_eligible: bool,
    ) -> anyhow::Result<crate::application::ports::ordinary_claims::ExecutionClaim> {
        use crate::application::ports::ordinary_claims::ExecutionClaim;
        let anchor = execution.reservation();
        let mut entry = self
            .keys
            .get_mut(&(anchor.wallet_id().into(), anchor.idempotency_key().into()))
            .ok_or_else(|| anyhow::anyhow!("ordinary execution key reservation missing"))?;
        anyhow::ensure!(
            entry.request_id == anchor.request_id()
                && entry.request_hash == anchor.request_hash()
                && entry.aggregator_sol,
            "ordinary execution key reservation mismatch"
        );
        if entry.execution_owner.is_some() {
            return Ok(ExecutionClaim::AlreadyHeld);
        }
        anyhow::ensure!(
            parent_eligible,
            "ordinary trade is not eligible for execution claim"
        );
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let owner = crate::application::ports::ordinary_claims::ExecutionOwner::new(
            execution,
            nonce.clone(),
        )?;
        entry.execution_owner = Some(nonce.clone());
        Ok(ExecutionClaim::Acquired(owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_fingerprint_and_path_replays_but_differences_conflict() {
        let store = IdempotencyStore::default();
        assert!(matches!(
            store.reserve("wallet", "key", "original", "legacy-hash", true),
            ReserveResult::Reserved
        ));
        assert!(
            matches!(store.reserve("wallet","key","repeat","legacy-hash",true),ReserveResult::Duplicate(id) if id=="original")
        );
        assert!(matches!(
            store.reserve("wallet", "key", "other-path", "legacy-hash", false),
            ReserveResult::Conflict
        ));
        assert!(matches!(
            store.reserve("wallet", "key", "other-params", "different-hash", true),
            ReserveResult::Conflict
        ));
        assert!(
            matches!(store.reserve("wallet","key","repeat-after-conflict","legacy-hash",true),ReserveResult::Duplicate(id) if id=="original")
        );
    }

    #[test]
    fn wallet_and_literal_key_are_separate_namespace_components() {
        let store = IdempotencyStore::default();
        for (wallet, key, id) in [
            ("a:b", "c", "one"),
            ("a", "b:c", "two"),
            ("a:b", " c ", "three"),
        ] {
            assert!(matches!(
                store.reserve(wallet, key, id, "hash", false),
                ReserveResult::Reserved
            ));
            assert!(
                matches!(store.reserve(wallet,key,"repeat","hash",false),ReserveResult::Duplicate(original) if original==id)
            );
        }
    }

    #[tokio::test]
    async fn concurrent_paths_cannot_overwrite_each_others_reservation() {
        let store = std::sync::Arc::new(IdempotencyStore::default());
        let mut tasks = vec![];
        for index in 0..100 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                store.reserve(
                    "wallet",
                    "key",
                    &format!("request-{index}"),
                    "same-old-hash",
                    index % 2 == 0,
                )
            }));
        }
        let (mut created, mut duplicates, mut conflicts) = (0, 0, 0);
        for task in tasks {
            match task.await.unwrap() {
                ReserveResult::Reserved => created += 1,
                ReserveResult::Duplicate(_) => duplicates += 1,
                ReserveResult::Conflict => conflicts += 1,
            }
        }
        assert_eq!((created, duplicates, conflicts), (1, 49, 50));
    }
}
