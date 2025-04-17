use crate::broker::Broker;
use crate::message::EndpointHistory;
use crate::runnable::Runnable;
use async_trait::async_trait;
use log::trace;
use std::collections::HashMap;
use tokio::sync::Mutex;

/// Prefix for aggregator instance names
const AGGREGATOR_NAME_PREFIX: &str = "Aggregator";

/// Aggregator responsible for processing monitoring results
///
/// The Aggregator component:
/// - Receives results from workers
/// - Processes results and handles retries
/// - Manages the result processing lifecycle
///
/// # Fields
/// * `name` - Name of the aggregator instance
/// * `result_receiver` - Channel for receiving results from workers
/// * `shutdown_receiver` - Receiver for shutdown signals
pub struct Aggregator {
    name: String,
    broker: Broker,
    pool: Mutex<HashMap<u32, EndpointHistory>>,
}

impl Aggregator {
    /// Creates a new Aggregator instance
    ///
    /// # Arguments
    /// * `id` - Unique identifier for this aggregator
    /// * `result_receiver` - Channel for receiving results from workers
    /// * `shutdown_receiver` - Receiver for shutdown signals
    pub fn new(id: usize, broker: Broker) -> Self {
        Self {
            name: format!("{}-{}", AGGREGATOR_NAME_PREFIX, id),
            broker,
            pool: Mutex::new(HashMap::new()),
        }
    }

    /// Continuously receives and processes results from workers
    ///
    /// This method:
    /// 1. Receives results from the worker channel
    /// 2. Processes each result and logs the outcome
    async fn process_results(&self) {
        if let Some(result) = self.broker.receive_result().await {
            // release the lock as soon as possible
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
            trace!("{:?}", history);
            history.events.push(result.record);

            if let Some(report) = self.collect_recent_failure(history) {
                let _ = self.broker.send_report(report).await;
            }
        }
    }

    /// Check if endpoint failed failure_threshold times,
    /// if so, send the last failure_threshold events to the broker
    pub fn collect_recent_failure(&self, history: &EndpointHistory) -> Option<EndpointHistory> {
        let failure_threshold = history.endpoint.failure_threshold as usize;
        let events = &history.events;
        if events.len() >= failure_threshold {
            let recent_failures: Vec<_> = events
                .iter()
                .rev()
                .take(failure_threshold)
                .cloned()
                .collect();
            return Some(EndpointHistory {
                endpoint: history.endpoint.clone(),
                events: recent_failures.into_iter().rev().collect(),
            });
        }
        None
    }
}

#[async_trait]
impl Runnable for Aggregator {
    async fn run(&self) {
        self.process_results().await;
    }

    /// Returns the name of this aggregator instance
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

    fn make_endpoint(failure_threshold: u8) -> Endpoint {
        Endpoint {
            id: 0,
            url: "http://test".to_string(),
            failure_threshold,
            timeout: Duration::from_secs(1),
        }
    }

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

    #[tokio::test]
    async fn test_not_enough_failures() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = [make_event(
            &endpoint,
            StatusCode::INTERNAL_SERVER_ERROR,
            now,
        )];
        let history = EndpointHistory {
            endpoint: endpoint.clone(),
            events: events.iter().map(|e| e.record.clone()).collect(),
        };
        let aggregator = Aggregator::new(0, Broker::new());
        let result = aggregator.collect_recent_failure(&history);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_enough_failures() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        // 創建間隔較長的事件，但不檢查時間窗口
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
        let history = EndpointHistory {
            endpoint: endpoint.clone(),
            events: events.iter().map(|e| e.record.clone()).collect(),
        };
        let aggregator = Aggregator::new(0, Broker::new());
        let result = aggregator.collect_recent_failure(&history);
        assert!(result.is_some());
        let reported = result.unwrap();
        assert_eq!(reported.events.len(), 3);
        assert_eq!(
            reported.events,
            events.iter().map(|e| e.record.clone()).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_event_order_preserved() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        // 確保事件順序被保留
        let events = vec![
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
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
        ];
        let history = EndpointHistory {
            endpoint: endpoint.clone(),
            events: events.iter().map(|e| e.record.clone()).collect(),
        };
        let aggregator = Aggregator::new(0, Broker::new());
        let result = aggregator.collect_recent_failure(&history);
        assert!(result.is_some());
        let reported = result.unwrap();

        // 確認事件順序是按時間戳排序的
        for i in 1..reported.events.len() {
            assert!(reported.events[i - 1].timestamp <= reported.events[i].timestamp);
        }

        assert_eq!(reported.events.len(), 3);
        assert_eq!(
            reported.events,
            events.iter().map(|e| e.record.clone()).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_failure_event_being_inserted() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = vec![
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
        ];

        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        for event in events {
            let _ = broker.send_result(event).await;
            aggregator.process_results().await;
        }

        assert_eq!(aggregator.pool.lock().await.len(), 1);
        assert_eq!(
            aggregator
                .pool
                .lock()
                .await
                .get(&endpoint.id)
                .unwrap()
                .events
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn test_success_event_clears_history() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = vec![
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
            make_event(&endpoint, StatusCode::OK, now),
        ];

        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        for event in events {
            let _ = broker.send_result(event).await;
            aggregator.process_results().await;
        }

        assert_eq!(aggregator.pool.lock().await.len(), 1);
        assert_eq!(
            aggregator
                .pool
                .lock()
                .await
                .get(&endpoint.id)
                .unwrap()
                .events
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn test_more_than_threshold_failures() {
        let endpoint = make_endpoint(2);
        let now = Utc::now();
        // 測試超過閾值的情況
        let events = vec![
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(40),
            ),
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
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(10),
            ),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
        ];
        let history = EndpointHistory {
            endpoint: endpoint.clone(),
            events: events.iter().map(|e| e.record.clone()).collect(),
        };
        let aggregator = Aggregator::new(0, Broker::new());
        let result = aggregator.collect_recent_failure(&history);
        assert!(result.is_some());
        let reported = result.unwrap();

        // 應該只取最近的 failure_threshold 次失敗
        assert_eq!(reported.events.len(), 2);

        // 確認取得的是最近的事件
        assert_eq!(
            reported.events[0].timestamp,
            now - ChronoDuration::seconds(10)
        );
        assert_eq!(reported.events[1].timestamp, now);
    }

    #[tokio::test]
    async fn test_endpoint_update() {
        // 創建初始 endpoint
        let initial_endpoint = Endpoint {
            id: 0,
            url: "http://test".to_string(),
            failure_threshold: 3,
            timeout: Duration::from_secs(1),
        };

        // 創建更新後的 endpoint (增加了 failure_threshold)
        let updated_endpoint = Endpoint {
            id: 0,
            url: "http://test".to_string(),
            failure_threshold: 5, // 從 3 變為 5
            timeout: Duration::from_secs(1),
        };

        let now = Utc::now();

        // 使用初始 endpoint 建立事件
        let initial_event = make_event(&initial_endpoint, StatusCode::INTERNAL_SERVER_ERROR, now);

        // 使用更新後的 endpoint 建立事件
        let updated_event = make_event(
            &updated_endpoint,
            StatusCode::INTERNAL_SERVER_ERROR,
            now + ChronoDuration::seconds(10),
        );

        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        // 發送初始事件
        let _ = broker.send_result(initial_event).await;
        aggregator.process_results().await;

        // 確認 pool 中的 endpoint 是初始版本
        {
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&initial_endpoint.id).unwrap();
            assert_eq!(history.endpoint.url, initial_endpoint.url);
            assert_eq!(history.endpoint.failure_threshold, 3);
            assert_eq!(history.endpoint.timeout, initial_endpoint.timeout);
        }

        // 發送更新後的事件
        let _ = broker.send_result(updated_event).await;
        aggregator.process_results().await;

        // 確認 pool 中的 endpoint 已更新
        {
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&initial_endpoint.id).unwrap();
            assert_eq!(history.endpoint.failure_threshold, 5); // 應該更新為 5
            assert_eq!(history.endpoint.url, updated_endpoint.url);
            assert_eq!(history.endpoint.timeout, updated_endpoint.timeout);
        }
    }
}
