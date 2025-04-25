use crate::{
    broker::{Broker, ReportSender, ResultReceiver},
    message::{EndpointHistory, QueryRecord, QueryResult},
    runnable::Runnable,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::{collections::HashMap, sync::Weak};
use tokio::sync::Mutex;

/// Prefix for aggregator instance names
const AGGREGATOR_NAME_PREFIX: &str = "Aggregator";

/// Aggregator responsible for processing monitoring results and detecting failure patterns.
///
/// The Aggregator component:
/// - Receives `QueryResult`s from workers via the `Broker`.
/// - Maintains a history of recent query events for each endpoint (`EndpointHistory`).
/// - If a result is successful, it clears the history for that endpoint.
/// - If a result indicates a failure, it adds the `QueryRecord` to the history.
/// - It retains only the most recent `failure_threshold` events in the history.
/// - If the number of consecutive failures reaches the `failure_threshold`,
///   it sends an `EndpointHistory` report to the `Broker`'s report channel
///   and clears the history for that endpoint.
///
/// # Fields
/// * `name` - Name of the aggregator instance (e.g., "Aggregator-0").
/// * `result_rx` - Receiver for QueryResults from the Broker.
/// * `report_tx` - Sender for EndpointHistory reports to the Broker.
/// * `pool` - A thread-safe map (`Mutex<HashMap<u32, EndpointHistory>>`) storing the recent event history for each monitored endpoint ID.
pub struct Aggregator {
    name: String,
    result_rx: ResultReceiver,
    report_tx: ReportSender,
    pool: Mutex<HashMap<u32, EndpointHistory>>,
}

impl Aggregator {
    /// Helper function to retain only the most recent `threshold` events in a vector.
    /// Removes older events from the beginning of the vector if the length exceeds the threshold.
    fn retain_recent_events(events: &mut Vec<QueryRecord>, threshold: usize) {
        if events.len() > threshold {
            let drain_count = events.len() - threshold;
            events.drain(0..drain_count);
        }
    }

    /// Processes a single `QueryResult` received from a worker.
    ///
    /// This method:
    /// 1. Locks the history pool.
    /// 2. Gets or creates the `EndpointHistory` for the result's endpoint ID.
    /// 3. If the result is successful, clears the history events for that endpoint.
    /// 4. If the result is a failure:
    ///    a. Updates the `Endpoint` information in the history (in case it changed).
    ///    b. Appends the `QueryRecord` to the history events.
    ///    c. Uses `retain_recent_events` to keep only the last `failure_threshold` events.
    ///    d. If the number of events now equals `failure_threshold`, sends a report using `report_tx` and clears the history events.
    async fn process_results(&self, result: QueryResult) {
        let mut pool = self.pool.lock().await;
        let history = pool.entry(result.endpoint.id).or_insert(EndpointHistory {
            endpoint: result.endpoint.clone(),
            events: Vec::new(),
        });

        if result.record.status.is_success() {
            history.events.clear();
            return;
        }

        // Update endpoint information as it might have changed
        history.endpoint = result.endpoint;
        history.events.push(result.record);

        let threshold = history.endpoint.failure_threshold as usize;
        Self::retain_recent_events(&mut history.events, threshold);

        if history.events.len() == threshold {
            if let Err(e) = self.report_tx.send(history.clone()) {
                tracing::warn!("{}: Failed to send report: {}", self.name, e);
            }
            history.events.clear();
        }
    }
}

#[async_trait]
impl Runnable for Aggregator {
    /// Creates a new Aggregator instance, compatible with the Runnable trait.
    fn new(id: usize, broker: Weak<Broker>) -> Result<Self> {
        if let Some(broker) = broker.upgrade() {
            Ok(Self {
                name: format!("{}-{}", AGGREGATOR_NAME_PREFIX, id),
                result_rx: broker.register_result_receiver(),
                report_tx: broker.register_report_sender(),
                pool: Mutex::new(HashMap::new()),
            })
        } else {
            Err(anyhow::anyhow!("Broker is not alive"))
        }
    }

    /// The main loop for the aggregator.
    /// Continuously receives `QueryResult`s from the broker's result channel.
    /// For each result, it calls `process_results`.
    /// The loop terminates when the broker's result channel is closed or a
    /// shutdown signal is received.
    async fn run(&mut self) {
        loop {
            tokio::select! {
                Some(result) = self.result_rx.receive() => {
                    self.process_results(result).await;
                }
                _ = self.report_tx.is_shutdown() => {
                    tracing::info!("{}: Report channel closed, shutting down.", self.name);
                    break;
                }
                else => {
                    tracing::info!("{}: Result channel closed, shutting down.", self.name);
                    break;
                }
            }
        }
    }

    /// Returns the name of this aggregator instance.
    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        broker::{Broker, ReportReceiver},
        message::{Endpoint, QueryRecord, QueryResult},
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use pretty_assertions::assert_eq;
    use reqwest::StatusCode;
    use std::{sync::Arc, time::Duration};

    /// Test helper structure, encapsulating common test setup.
    struct TestContext {
        aggregator: Aggregator,
        report_rx: ReportReceiver,
    }

    impl TestContext {
        /// Create a new test context.
        fn new() -> Self {
            let broker = Arc::new(Broker::new());
            let report_rx = broker.register_report_receiver();
            let aggregator = Aggregator::new(0, Arc::downgrade(&broker))
                .expect("Failed to create aggregator for test");

            Self {
                aggregator,
                report_rx,
            }
        }

        /// Process an event and return the history from the pool.
        async fn process_event(&mut self, event: QueryResult) -> EndpointHistory {
            self.aggregator.process_results(event.clone()).await;
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let pool = self.aggregator.pool.lock().await;
            pool.get(&event.endpoint.id)
                .cloned()
                .unwrap_or_else(|| EndpointHistory {
                    endpoint: event.endpoint.clone(),
                    events: Vec::new(),
                })
        }

        /// Check if a report was sent, using a timeout to avoid indefinite waiting.
        async fn check_report_sent(
            &mut self,
            should_send: bool,
            timeout_ms: u64,
        ) -> Option<EndpointHistory> {
            let timeout_result = tokio::time::timeout(
                tokio::time::Duration::from_millis(timeout_ms),
                self.report_rx.receive(),
            )
            .await;

            match timeout_result {
                Ok(Ok(report)) => {
                    if !should_send {
                        panic!("Report should not have been sent");
                    }
                    Some(report)
                }
                Ok(Err(_e)) => {
                    if should_send {
                        panic!("Report should have been sent, but receive failed/channel closed");
                    }
                    None
                }
                Err(_) => {
                    if should_send {
                        panic!("Report should have been sent, but timed out");
                    }
                    None
                }
            }
        }

        /// Check the number of events in the pool for a specific endpoint ID.
        async fn check_pool_events_count(&self, endpoint_id: u32, expected_count: usize) {
            let pool = self.aggregator.pool.lock().await;
            let history = pool.get(&endpoint_id);
            assert_eq!(history.map_or(0, |h| h.events.len()), expected_count);
        }
    }

    /// Helper function to create a test Endpoint.
    fn make_endpoint(failure_threshold: u8) -> Endpoint {
        Endpoint {
            id: 0,
            url: "http://test".to_string(),
            failure_threshold,
            timeout: Duration::from_secs(1),
        }
    }

    /// Helper function to create a test QueryResult (event).
    fn make_event(
        endpoint: &Endpoint,
        status: StatusCode,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> QueryResult {
        QueryResult {
            endpoint: endpoint.clone(),
            record: QueryRecord {
                status,
                timestamp,
                duration: Duration::from_millis(10),
            },
        }
    }

    /// Test case: Not enough failures to trigger a report.
    #[tokio::test]
    async fn test_not_enough_failures() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = [make_event(
            &endpoint,
            StatusCode::INTERNAL_SERVER_ERROR,
            now,
        )];

        let mut ctx = TestContext::new();

        for (i, event) in events.iter().enumerate() {
            let history = ctx.process_event(event.clone()).await;

            assert_eq!(history.events.len(), i + 1);

            assert_eq!(history.endpoint.id, endpoint.id);
            assert_eq!(history.endpoint.url, endpoint.url);
            assert_eq!(history.endpoint.failure_threshold, endpoint.failure_threshold);

            if i == 0 {
                assert_eq!(history.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(history.events[0].timestamp, now);
            }
        }

        ctx.check_pool_events_count(endpoint.id, 1).await;

        ctx.check_report_sent(false, 100).await;
    }

    /// Test case: Enough failures to trigger a report.
    #[tokio::test]
    async fn test_enough_failures() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = vec![
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(120),
            ),
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(60),
            ),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
        ];

        let mut ctx = TestContext::new();

        for (i, event) in events.iter().enumerate() {
            let history = ctx.process_event(event.clone()).await;

            if i < 2 {
                assert_eq!(history.events.len(), i + 1);
            } else {
                assert!(history.events.is_empty());
            }
        }

        let report = ctx.check_report_sent(true, 100).await.unwrap();

        assert_eq!(report.endpoint.id, endpoint.id);
        assert_eq!(report.events.len(), 3);
        assert_eq!(report.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[0].timestamp, now - ChronoDuration::seconds(120));
        assert_eq!(report.events[1].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[1].timestamp, now - ChronoDuration::seconds(60));
        assert_eq!(report.events[2].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[2].timestamp, now);

        ctx.check_pool_events_count(endpoint.id, 0).await;
    }

    /// Test case: Ensure events are stored in the correct order (before threshold).
    #[tokio::test]
    async fn test_event_order_preserved() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = [
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(30),
            ),
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(20),
            ),
        ];

        let mut ctx = TestContext::new();

        for (i, event) in events.iter().enumerate() {
            ctx.process_event(event.clone()).await;

            let pool = ctx.aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();
            assert_eq!(history.events.len(), i + 1);
            assert_eq!(history.events[i].status, event.record.status);
            assert_eq!(history.events[i].timestamp, event.record.timestamp);
        }
        ctx.check_report_sent(false, 50).await;
    }

    /// Test case: Handling of threshold limits and event overflow.
    #[tokio::test]
    async fn test_threshold_and_overflow_handling() {
        let endpoint = make_endpoint(2);
        let now = Utc::now();
        let events = vec![
            make_event(&endpoint, StatusCode::BAD_GATEWAY, now - ChronoDuration::seconds(3)),
            make_event(&endpoint, StatusCode::GATEWAY_TIMEOUT, now - ChronoDuration::seconds(2)),
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(1),
            ),
            make_event(&endpoint, StatusCode::NOT_FOUND, now),
        ];

        let mut ctx = TestContext::new();

        ctx.process_event(events[0].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;
        ctx.check_report_sent(false, 50).await;

        ctx.process_event(events[1].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 0).await;
        let report1 = ctx.check_report_sent(true, 50).await.unwrap();
        assert_eq!(report1.events.len(), 2);
        assert_eq!(report1.events[0].status, StatusCode::BAD_GATEWAY);
        assert_eq!(report1.events[1].status, StatusCode::GATEWAY_TIMEOUT);

        ctx.process_event(events[2].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;
        ctx.check_report_sent(false, 50).await;

        ctx.process_event(events[3].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 0).await;
        let report2 = ctx.check_report_sent(true, 50).await.unwrap();
        assert_eq!(report2.events.len(), 2);
        assert_eq!(report2.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report2.events[1].status, StatusCode::NOT_FOUND);
    }

    /// Test case: Ensures that only failure events are inserted into the history.
    #[tokio::test]
    async fn test_failure_event_being_inserted() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let success_event = make_event(&endpoint, StatusCode::OK, now - ChronoDuration::seconds(1));
        let failure_event = make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now);

        let mut ctx = TestContext::new();

        ctx.process_event(success_event).await;
        ctx.check_pool_events_count(endpoint.id, 0).await;

        ctx.process_event(failure_event).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;
    }

    /// Test case: Verifies that a success event clears existing failure history.
    #[tokio::test]
    async fn test_success_event_clears_history() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let failure_event = make_event(
            &endpoint,
            StatusCode::INTERNAL_SERVER_ERROR,
            now - ChronoDuration::seconds(1),
        );
        let success_event = make_event(&endpoint, StatusCode::OK, now);

        let mut ctx = TestContext::new();

        ctx.process_event(failure_event).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;

        ctx.process_event(success_event).await;
        ctx.check_pool_events_count(endpoint.id, 0).await;
        ctx.check_report_sent(false, 50).await;
    }

    /// Test case: Ensure endpoint details (like threshold) are updated with new events.
    #[tokio::test]
    async fn test_endpoint_update() {
        let endpoint1 = make_endpoint(3);
        let mut endpoint2 = endpoint1.clone();
        endpoint2.failure_threshold = 5;
        endpoint2.url = "http://updated-test".to_string();

        let now = Utc::now();
        let event1 = make_event(
            &endpoint1,
            StatusCode::INTERNAL_SERVER_ERROR,
            now - ChronoDuration::seconds(1),
        );
        let event2 = make_event(&endpoint2, StatusCode::SERVICE_UNAVAILABLE, now);

        let mut ctx = TestContext::new();

        ctx.process_event(event1).await;
        {
            let pool = ctx.aggregator.pool.lock().await;
            let history = pool.get(&endpoint1.id).unwrap();
            assert_eq!(history.endpoint.failure_threshold, 3);
            assert_eq!(history.endpoint.url, "http://test");
            assert_eq!(history.events.len(), 1);
        }

        let history_after_update = ctx.process_event(event2).await;

        assert_eq!(history_after_update.endpoint.failure_threshold, 5);
        assert_eq!(history_after_update.endpoint.url, "http://updated-test");
        assert_eq!(history_after_update.events.len(), 2);
        assert_eq!(history_after_update.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(history_after_update.events[1].status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
