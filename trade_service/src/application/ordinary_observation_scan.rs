use crate::{
    application::{
        ordinary_reconciliation::reconcile_captured,
        ports::{
            ordinary_reconciliation::RecoveredOrdinaryObservations,
            reconciliation::SignatureStatusReader,
        },
    },
    domain::trade::TradeStatus,
};
use anyhow::{ensure, Result};
use std::{future::Future, sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ObservationCursor {
    created_at_ms: i64,
    request_id: String,
}
impl ObservationCursor {
    pub(crate) fn new(created_at_ms: i64, request_id: String) -> Result<Self> {
        ensure!(
            created_at_ms >= 0
                && !request_id.is_empty()
                && request_id.len() <= 128
                && request_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "invalid observation cursor"
        );
        Ok(Self {
            created_at_ms,
            request_id,
        })
    }
    pub(crate) fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
}
pub(crate) struct ObservationPage {
    pub(crate) entries: Vec<ObservationCursor>,
    pub(crate) has_more: bool,
}
pub(crate) trait PendingOrdinaryObservations: Sync {
    fn pending_observations(
        &self,
        after: Option<&ObservationCursor>,
        cutoff_ms: i64,
        limit: usize,
    ) -> impl Future<Output = Result<ObservationPage>> + Send;
}

#[derive(Debug, Default)]
pub(crate) struct ScanReport {
    pub(crate) examined: usize,
    pub(crate) finalized: usize,
    pub(crate) pending: usize,
    pub(crate) unavailable: usize,
    pub(crate) rejected: usize,
    pub(crate) wrapped: bool,
}

pub(crate) struct OrdinaryObservationScanner<J, R> {
    journal: J,
    reader: Arc<R>,
    after: Option<ObservationCursor>,
    cutoff: Option<i64>,
}
impl<J: PendingOrdinaryObservations + RecoveredOrdinaryObservations, R: SignatureStatusReader>
    OrdinaryObservationScanner<J, R>
{
    pub(crate) fn new(journal: J, reader: Arc<R>) -> Self {
        Self {
            journal,
            reader,
            after: None,
            cutoff: None,
        }
    }
    pub(crate) async fn step(
        &mut self,
        cutoff_ms: i64,
        limit: usize,
        pace: Duration,
    ) -> Result<ScanReport> {
        ensure!(
            cutoff_ms >= 0 && (1..=64).contains(&limit),
            "invalid observation scan bounds"
        );
        let cutoff = *self.cutoff.get_or_insert(cutoff_ms);
        let page = self
            .journal
            .pending_observations(self.after.as_ref(), cutoff, limit)
            .await?;
        ensure!(
            page.entries.len() <= limit && (!page.has_more || !page.entries.is_empty()),
            "observation page exceeded bounds"
        );
        let mut previous = self.after.as_ref();
        for entry in &page.entries {
            ensure!(
                entry.created_at_ms() <= cutoff && previous.is_none_or(|old| old < entry),
                "observation page cursor is not increasing"
            );
            previous = Some(entry);
        }
        let mut report = ScanReport::default();
        for (index, entry) in page.entries.iter().enumerate() {
            match self.journal.capture_observation(entry.request_id()).await {
                Err(error) => {
                    report.rejected += 1;
                    tracing::warn!(request_id = %entry.request_id(), error = %crate::sanitize::redact_urls(&error.to_string()), "ordinary observation candidate rejected; no transaction retry");
                }
                Ok(captured) => {
                    if captured.context.request_id() != entry.request_id() {
                        report.rejected += 1;
                    } else {
                        match reconcile_captured(
                            &self.journal,
                            self.reader.as_ref(),
                            &captured.context,
                            pace.saturating_add(Duration::from_secs(5)),
                        )
                        .await
                        {
                            Ok(result) => {
                                if matches!(
                                    result.record.status,
                                    TradeStatus::Confirmed | TradeStatus::Failed
                                ) {
                                    report.finalized += 1;
                                } else {
                                    report.pending += 1;
                                }
                                if result.lookup_error.is_some() {
                                    report.unavailable += 1;
                                }
                            }
                            Err(error) => {
                                report.rejected += 1;
                                tracing::warn!(request_id = %entry.request_id(), error = %crate::sanitize::redact_urls(&error.to_string()), "ordinary observation unavailable; preserve original operation");
                            }
                        }
                    }
                }
            }
            // A cancelled batch retains progress, but never skips an unfinished observation.
            self.after = Some(entry.clone());
            report.examined += 1;
            if index + 1 < page.entries.len() {
                tokio::time::sleep(pace).await;
            }
        }
        if !page.has_more {
            self.after = None;
            self.cutoff = None;
            report.wrapped = true;
        }
        Ok(report)
    }
}
