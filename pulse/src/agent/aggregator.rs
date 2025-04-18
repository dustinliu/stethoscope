use crate::broker::Broker;
use crate::message::{EndpointHistory, QueryResult};
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

    /// Helper: retain only the most recent `threshold` events
    fn retain_recent_events(events: &mut Vec<crate::message::QueryRecord>, threshold: usize) {
        if events.len() > threshold {
            let drain_count = events.len() - threshold;
            events.drain(0..drain_count);
        }
    }

    /// Continuously receives and processes results from workers
    ///
    /// This method:
    /// 1. Receives results from the worker channel
    /// 2. Processes each result and logs the outcome
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
        trace!("{:?}", history);

        let threshold = history.endpoint.failure_threshold as usize;
        Self::retain_recent_events(&mut history.events, threshold);

        if history.events.len() == threshold {
            let report = EndpointHistory {
                endpoint: history.endpoint.clone(),
                events: history.events.clone(),
            };
            let _ = self.broker.send_report(report).await;
            history.events.clear();
        }
    }
}

#[async_trait]
impl Runnable for Aggregator {
    async fn run(&mut self) {
        while let Some(result) = self.broker.receive_result().await {
            self.process_results(result).await;

            if self.broker.is_shutdown() {
                break;
            }
        }
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
        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        // 創建一個任務監聽是否有 report 被送出
        let mut report_broker = broker.clone();
        let report_check = tokio::spawn(async move {
            // 使用短超時，避免永久等待
            tokio::time::timeout(
                tokio::time::Duration::from_millis(10),
                report_broker.receive_report(),
            )
            .await
        });

        for (i, event) in events.iter().enumerate() {
            aggregator.process_results(event.clone()).await;
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();

            // 檢查 event 數量
            assert_eq!(history.events.len(), i + 1);

            // 檢查 endpoint 資訊是否正確保存
            assert_eq!(history.endpoint.id, endpoint.id);
            assert_eq!(history.endpoint.url, endpoint.url);
            assert_eq!(
                history.endpoint.failure_threshold,
                endpoint.failure_threshold
            );

            // 檢查最後一個事件內容是否正確
            if i == 0 {
                assert_eq!(history.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);
                assert_eq!(history.events[0].timestamp, now);
            }
        }

        // 最後確認 pool 內仍存在 event（未達到 threshold）
        let pool = aggregator.pool.lock().await;
        let history = pool.get(&endpoint.id).unwrap();
        assert_eq!(history.events.len(), 1);
        assert_eq!(history.events[0].status, StatusCode::INTERNAL_SERVER_ERROR);

        // 驗證沒有 report 被送出
        let report_result = report_check.await.expect("檢查任務失敗");
        assert!(report_result.is_err(), "報告不應該在 threshold 前送出");
    }

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
        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        for (i, event) in events.iter().enumerate() {
            if i < 2 {
                // 前兩個事件送出時不應該有 report
                let mut report_broker = broker.clone();
                let report_check = tokio::spawn(async move {
                    // 使用 10ms 超時避免永久等待

                    tokio::time::timeout(
                        tokio::time::Duration::from_millis(10),
                        report_broker.receive_report(),
                    )
                    .await
                });

                // 送入事件
                aggregator.process_results(event.clone()).await;

                // 檢查 pool 內容
                let pool = aggregator.pool.lock().await;
                let history = pool.get(&endpoint.id).unwrap();
                assert_eq!(history.events.len(), i + 1);

                // 確認沒有 report 被送出
                let report_result = report_check.await.expect("檢查任務失敗");
                assert!(report_result.is_err(), "報告不應該在 threshold 前送出");
            } else {
                // 第三個事件應該觸發 report
                let mut report_broker = broker.clone();
                let report_receiver =
                    tokio::spawn(async move { report_broker.receive_report().await });

                // 送入事件
                aggregator.process_results(event.clone()).await;

                // 檢查 pool 被清空
                let pool = aggregator.pool.lock().await;
                let history = pool.get(&endpoint.id).unwrap();
                assert_eq!(history.events.len(), 0);

                // 驗證 report 被送出
                let report = report_receiver.await.expect("接收任務失敗");
                assert!(report.is_some(), "達到 threshold 應該送出報告");
                let report = report.unwrap();
                assert_eq!(report.events.len(), 3);
                assert_eq!(report.endpoint.id, endpoint.id);
            }
        }
    }

    #[tokio::test]
    async fn test_event_order_preserved() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
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
        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());
        for (i, event) in events.iter().enumerate() {
            aggregator.process_results(event.clone()).await;
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();
            // 累積到 threshold 就會清空
            let expected = if i == 2 { 0 } else { i + 1 };
            assert_eq!(history.events.len(), expected);
        }
    }

    #[tokio::test]
    async fn test_threshold_and_overflow_handling() {
        // 使用 failure_threshold=2 的 endpoint
        let endpoint = make_endpoint(2);
        let now = Utc::now();

        // 創建 5 個事件，模擬：
        // 1-2: 達到閾值，發送第一次報告
        // 3-4: 達到閾值，發送第二次報告
        // 5: 未達閾值，保留在 pool
        let events = vec![
            make_event(
                &endpoint,
                StatusCode::INTERNAL_SERVER_ERROR,
                now - ChronoDuration::seconds(50),
            ),
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
        ];

        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        // 期望的 pool 長度
        // 第一輪：1個→0個（達到閾值）
        // 第二輪：1個→0個（達到閾值）
        // 第三輪：1個（未達閾值）
        let expected_pool_lens = [1, 0, 1, 0, 1];

        // 標記預期會發送報告的事件索引
        let report_events = [1, 3]; // 第2個和第4個事件（索引 1 和 3）時報告會被發送

        for (i, event) in events.iter().enumerate() {
            // 送入事件
            aggregator.process_results(event.clone()).await;

            // 檢查 pool 長度
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();
            assert_eq!(
                history.events.len(),
                expected_pool_lens[i],
                "第 {} 個事件後，pool 長度應為 {}",
                i + 1,
                expected_pool_lens[i]
            );
            drop(pool);

            // 檢查報告
            let mut report_broker = broker.clone();
            if report_events.contains(&i) {
                // 此事件應該觸發報告
                let report = report_broker.receive_report().await;
                assert!(report.is_some(), "第 {} 個事件應觸發報告", i + 1);

                if let Some(report) = report {
                    assert_eq!(report.endpoint.id, endpoint.id);
                    assert_eq!(report.events.len(), 2, "報告應包含 2 個事件");

                    // 檢查事件時間戳順序
                    assert!(report.events[0].timestamp < report.events[1].timestamp);

                    // 檢查是我們期望的事件
                    let expected_start = if i == 1 { 0 } else { 2 };
                    let expected_first_timestamp = events[expected_start].record.timestamp;
                    assert_eq!(report.events[0].timestamp, expected_first_timestamp);
                }
            } else {
                // 此事件不應該觸發報告，使用超時檢查
                let timeout_result = tokio::time::timeout(
                    tokio::time::Duration::from_millis(10),
                    report_broker.receive_report(),
                )
                .await;
                assert!(timeout_result.is_err(), "第 {} 個事件不應觸發報告", i + 1);
            }
        }
    }

    #[tokio::test]
    async fn test_failure_event_being_inserted() {
        let endpoint = make_endpoint(3);
        let now = Utc::now();
        let events = [
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
            make_event(&endpoint, StatusCode::INTERNAL_SERVER_ERROR, now),
        ];

        let broker = Broker::new();
        let aggregator = Aggregator::new(0, broker.clone());

        let expected_pool_lens = [1, 2, 0]; // 累積到 3 個時，清空 pool
        for (i, event) in events.iter().enumerate() {
            aggregator.process_results(event.clone()).await;
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&endpoint.id).unwrap();
            assert_eq!(history.events.len(), expected_pool_lens[i]);
        }

        assert_eq!(aggregator.pool.lock().await.len(), 1); // 仍然有一個 endpoint
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
            let _ = aggregator.process_results(event).await;
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

        aggregator.process_results(initial_event).await;

        {
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&initial_endpoint.id).unwrap();
            assert_eq!(history.endpoint.url, initial_endpoint.url);
            assert_eq!(history.endpoint.failure_threshold, 3);
            assert_eq!(history.endpoint.timeout, initial_endpoint.timeout);
        }

        aggregator.process_results(updated_event).await;

        {
            let pool = aggregator.pool.lock().await;
            let history = pool.get(&initial_endpoint.id).unwrap();
            assert_eq!(history.endpoint.failure_threshold, 5); // 應該更新為 5
            assert_eq!(history.endpoint.url, updated_endpoint.url);
            assert_eq!(history.endpoint.timeout, updated_endpoint.timeout);
        }
    }
}
