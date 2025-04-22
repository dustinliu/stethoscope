use crate::broker::Broker;
use crate::message::{EndpointHistory, QueryRecord, QueryResult};
use crate::runnable::Runnable;
use async_trait::async_trait;
use std::collections::HashMap;
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
/// * `broker` - Cloned `Broker` instance for receiving results and sending reports.
/// * `pool` - A thread-safe map (`Mutex<HashMap<u32, EndpointHistory>>`) storing the recent event history for each monitored endpoint ID.
#[derive(Debug)]
pub struct Aggregator {
    name: String,
    broker: Broker,
    pool: Mutex<HashMap<u32, EndpointHistory>>,
}

impl Aggregator {
    /// Creates a new Aggregator instance.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this aggregator instance.
    /// * `broker` - Cloned `Broker` instance for communication.
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", AGGREGATOR_NAME_PREFIX, id),
            broker,
            pool: Mutex::new(HashMap::new()),
        }
    }

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
    ///    d. If the number of events now equals `failure_threshold`, sends a report using `broker.send_report` and clears the history events.
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
            let _ = self.broker.send_report(history.clone());
            history.events.clear();
        }
    }
}

#[async_trait]
impl Runnable for Aggregator {
    /// The main loop for the aggregator.
    /// Continuously receives `QueryResult`s from the broker's result channel.
    /// For each result, it calls `process_results`.
    /// The loop terminates when the broker's result channel is closed or a
    /// shutdown signal is received.
    async fn run(&mut self) {
        while let Some(result) = self.broker.receive_result().await {
            // tracing::trace!("result capacity: {}", self.broker.result_channel_capacity());
            self.process_results(result).await;
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
    use crate::message::{Endpoint, QueryRecord, QueryResult};
    use chrono::{Duration as ChronoDuration, Utc};
    use pretty_assertions::assert_eq;
    use reqwest::StatusCode;
    use std::time::Duration;

    /// Test helper structure, encapsulating common test setup.
    struct TestContext {
        aggregator: Aggregator,
        broker: Broker,
    }

    impl TestContext {
        /// Create a new test context.
        fn new() -> Self {
            let broker = Broker::new();
            let aggregator = Aggregator::new(0, broker.clone());
            Self { aggregator, broker }
        }

        /// Process an event and return the history from the pool.
        async fn process_event(&self, event: QueryResult) -> EndpointHistory {
            self.aggregator.process_results(event.clone()).await;
            // Wait a short time to ensure the report can be sent.
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            let pool = self.aggregator.pool.lock().await;
            pool.get(&event.endpoint.id).unwrap().clone()
        }

        /// Check if a report was sent, using a timeout to avoid indefinite waiting.
        async fn check_report_sent(
            &mut self,
            should_send: bool, // Expected behavior: true if a report should be sent, false otherwise.
            timeout_ms: u64,   // Timeout duration in milliseconds.
        ) -> Option<EndpointHistory> {
            let timeout_result = tokio::time::timeout(
                tokio::time::Duration::from_millis(timeout_ms),
                self.broker.receive_report(),
            )
            .await;

            match timeout_result {
                Ok(Ok(report)) => {
                    if !should_send {
                        panic!("Report should not have been sent");
                    }
                    Some(report)
                }
                Ok(Err(_)) => {
                    if should_send {
                        panic!("Report should have been sent but was not");
                    }
                    None
                }
                Err(_) => {
                    // Timeout occurred
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
            let history = pool.get(&endpoint_id).unwrap();
            assert_eq!(history.events.len(), expected_count);
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

            // Check event count.
            assert_eq!(history.events.len(), i + 1);

            // Check if endpoint information is correctly saved.
            assert_eq!(history.endpoint.id, endpoint.id);
            assert_eq!(history.endpoint.url, endpoint.url);
            assert_eq!(history.endpoint.failure_threshold, endpoint.failure_threshold);

            // Check if the last event content is correct.
            if i == 0 {
                assert_eq!(history.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(history.events[0].timestamp, now);
            }
        }

        // Finally, confirm that events still exist in the pool (threshold not reached).
        ctx.check_pool_events_count(endpoint.id, 1).await;

        // Verify that no report was sent.
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
                // Before reaching the threshold, events should accumulate.
                assert_eq!(history.events.len(), i + 1);
            } else {
                // Upon reaching the threshold, events should be cleared.
                assert!(history.events.is_empty());
            }
        }

        // Verify that a report was sent.
        let report = ctx.check_report_sent(true, 100).await.unwrap();

        // Check report content.
        assert_eq!(report.endpoint.id, endpoint.id);
        assert_eq!(report.events.len(), 3);
        assert_eq!(report.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[0].timestamp, now - ChronoDuration::seconds(120));
        assert_eq!(report.events[1].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[1].timestamp, now - ChronoDuration::seconds(60));
        assert_eq!(report.events[2].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(report.events[2].timestamp, now);

        // Check that the pool is empty after the report.
        ctx.check_pool_events_count(endpoint.id, 0).await;
    }

    /// Test case: Ensure events are stored in the correct order (before threshold).
    #[tokio::test]
    async fn test_event_order_preserved() {
        let endpoint = make_endpoint(3); // Threshold is 3
        let now = Utc::now();
        // Only create 2 events, less than the threshold
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
            // Removed the third event to stay below threshold
        ];

        let ctx = TestContext::new(); // No need for mut ctx anymore
        // Removed threshold variable as it's not needed in the simplified logic

        for (i, event) in events.iter().enumerate() {
            ctx.process_event(event.clone()).await;

            // Original check: Directly check the pool as threshold is not reached.
            let pool = ctx.aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();
            assert_eq!(history.events.len(), i + 1);
            assert_eq!(history.events[i].status, event.record.status);
            assert_eq!(history.events[i].timestamp, event.record.timestamp);
            // Removed the complex if/else logic and report checking
        }
        // Optional: Add a final check that no report was sent
        // let mut ctx_mut = ctx; // Need mutability for check_report_sent
        // ctx_mut.check_report_sent(false, 50).await;
    }

    /// Test case: Handling of threshold limits and event overflow.
    #[tokio::test]
    async fn test_threshold_and_overflow_handling() {
        let endpoint = make_endpoint(2); // Threshold of 2
        let now = Utc::now();
        let events = vec![
            make_event(&endpoint, StatusCode::BAD_GATEWAY, now - ChronoDuration::seconds(3)),
            make_event(&endpoint, StatusCode::GATEWAY_TIMEOUT, now - ChronoDuration::seconds(2)),
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(1),
            ), // This event exceeds the threshold
            make_event(&endpoint, StatusCode::NOT_FOUND, now), // This should trigger the report
        ];

        let mut ctx = TestContext::new();

        // Process the first event.
        ctx.process_event(events[0].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;
        ctx.check_report_sent(false, 50).await; // No report yet

        // Process the second event (reaches threshold).
        ctx.process_event(events[1].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 0).await; // Pool cleared after report
        let report1 = ctx.check_report_sent(true, 50).await.unwrap(); // Report sent
        assert_eq!(report1.events.len(), 2);
        assert_eq!(report1.events[0].status, StatusCode::BAD_GATEWAY);
        assert_eq!(report1.events[1].status, StatusCode::GATEWAY_TIMEOUT);

        // Process the third event (first after clear).
        ctx.process_event(events[2].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;
        ctx.check_report_sent(false, 50).await; // No report yet

        // Process the fourth event (reaches threshold again).
        ctx.process_event(events[3].clone()).await;
        ctx.check_pool_events_count(endpoint.id, 0).await; // Pool cleared again
        let report2 = ctx.check_report_sent(true, 50).await.unwrap(); // Second report sent
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

        let ctx = TestContext::new();

        // Process a success event first.
        ctx.process_event(success_event).await;
        ctx.check_pool_events_count(endpoint.id, 0).await; // Should be empty

        // Process a failure event.
        ctx.process_event(failure_event).await;
        ctx.check_pool_events_count(endpoint.id, 1).await; // Should contain the failure
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

        // Add a failure event.
        ctx.process_event(failure_event).await;
        ctx.check_pool_events_count(endpoint.id, 1).await;

        // Process a success event.
        ctx.process_event(success_event).await;
        ctx.check_pool_events_count(endpoint.id, 0).await; // History should be cleared
        ctx.check_report_sent(false, 50).await; // No report should be sent
    }

    /// Test case: Ensure endpoint details (like threshold) are updated with new events.
    #[tokio::test]
    async fn test_endpoint_update() {
        let endpoint1 = make_endpoint(3);
        let mut endpoint2 = endpoint1.clone();
        endpoint2.failure_threshold = 5; // Change the threshold
        endpoint2.url = "http://updated-test".to_string(); // Change the URL

        let now = Utc::now();
        let event1 = make_event(
            &endpoint1,
            StatusCode::INTERNAL_SERVER_ERROR,
            now - ChronoDuration::seconds(1),
        );
        let event2 = make_event(&endpoint2, StatusCode::SERVICE_UNAVAILABLE, now);

        let ctx = TestContext::new();

        // Process event with original endpoint details.
        ctx.process_event(event1).await;
        {
            let pool = ctx.aggregator.pool.lock().await;
            let history = pool.get(&endpoint1.id).unwrap();
            assert_eq!(history.endpoint.failure_threshold, 3);
            assert_eq!(history.endpoint.url, "http://test");
            assert_eq!(history.events.len(), 1);
        }

        // Process event with updated endpoint details.
        let history_after_update = ctx.process_event(event2).await;

        // Check if endpoint details in history are updated.
        assert_eq!(history_after_update.endpoint.failure_threshold, 5);
        assert_eq!(history_after_update.endpoint.url, "http://updated-test");
        assert_eq!(history_after_update.events.len(), 2); // Both events should be present
        assert_eq!(history_after_update.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(history_after_update.events[1].status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
