use super::*;
use crate::adapters::rpc::{
    read_runtime::{quote_status_fixture as f, SharedReadRuntime},
    sdk_signature::observe_sdk_signature_shared,
};
use serde_json::json;

struct ObservingLane {
    lane: Lane,
    runtime: Arc<SharedReadRuntime<f::Fake>>,
}
impl OrdinarySdkLane for ObservingLane {
    type Prepared = ();
    fn provider(&self) -> SwqosType {
        self.lane.provider()
    }
    async fn prepare(&self, trade: &ExecuteTrade) -> Result<()> {
        self.lane.prepare(trade).await
    }
    async fn invoke(&self, prepared: ()) -> Result<SdkLaneReport, SdkInvocationFailure> {
        self.lane.invoke(prepared).await
    }
    async fn observe(&self, sig: &str, trade: &ExecuteTrade) -> Result<SdkLaneObservation> {
        self.lane.stage(Stage::Observe).await;
        Ok(
            match observe_sdk_signature_shared(
                &self.runtime,
                f::PRIMARY,
                sig,
                trade.mev_submit_verify_ms,
            )
            .await?
            {
                Some(observation) => SdkLaneObservation::Observed(observation),
                None => SdkLaneObservation::NotVisible,
            },
        )
    }
}
fn lanes(
    events: Arc<Mutex<Vec<(SwqosType, Stage)>>>,
    runtime: Arc<SharedReadRuntime<f::Fake>>,
) -> [ObservingLane; 2] {
    [SwqosType::Temporal, SwqosType::Default].map(|provider| ObservingLane {
        lane: Lane::new(provider, events.clone()),
        runtime: runtime.clone(),
    })
}
fn first_only(events: &Mutex<Vec<(SwqosType, Stage)>>) {
    assert_eq!(
        *events.lock().unwrap(),
        [
            (SwqosType::Temporal, Stage::Prepare),
            (SwqosType::Temporal, Stage::Invoke),
            (SwqosType::Temporal, Stage::Observe)
        ]
    );
}

#[tokio::test]
async fn shared_observer_flow_timeout_error_onchain_failure_and_late_receipt_preserve_original_unknown_signature(
) {
    let mut failed = f::found();
    failed["value"][0] = f::status("processed", json!({"InstructionError":[0,{"Custom":42}]}));
    for (reply, budget) in [
        (f::Reply::Hold, 30),
        (f::Reply::Fault(f::Fault::Rpc), 1500),
        (f::Reply::Good(failed), 1500),
        (f::Reply::Late(700, f::found()), 100),
    ] {
        let r = f::runtime(vec![(f::PRIMARY, vec![reply])]);
        let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
        let mut request = trade();
        request.mev_submit_verify_ms = budget;
        let (status, sig, error) = classify_result(
            execute(lanes(events.clone(), r.clone()), &request).await,
            false,
        );
        assert_eq!((status, sig), (TradeStatus::Unknown, Some(signature(1))));
        assert!(error.is_some());
        first_only(&events);
        assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
        assert_eq!(r.test_transport().calls.lock().unwrap()[0].0, f::PRIMARY);
        f::released(&r);
    }
}

#[tokio::test]
async fn shared_observer_flow_valid_original_receipt_is_submission_not_new_finality_or_another_lane(
) {
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Good(f::found())])]);
    let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
    let mut request = trade();
    request.mev_submit_verify_ms = 1500;
    let (status, sig, error) = classify_result(
        execute(lanes(events.clone(), r.clone()), &request).await,
        false,
    );
    assert_eq!(
        (status, sig, error),
        (TradeStatus::Submitted, Some(signature(1)), None)
    );
    first_only(&events);
    f::released(&r);
}

#[tokio::test]
async fn shared_observer_flow_caller_cancel_drops_local_observation_without_invoking_fallback() {
    let r = f::runtime(vec![(f::PRIMARY, vec![f::Reply::Hold])]);
    let events: Arc<Mutex<Vec<(SwqosType, Stage)>>> = Arc::default();
    let mut request = trade();
    request.mev_submit_verify_ms = 10_000;
    let mut pending = Box::pin(execute(lanes(events.clone(), r.clone()), &request));
    tokio::select! { result=&mut pending=>panic!("unexpected completion {}",result.is_ok()),_=r.test_transport().entered.notified()=>{} }
    drop(pending);
    first_only(&events);
    f::released(&r);
    assert_eq!(r.test_transport().calls.lock().unwrap().len(), 1);
}
