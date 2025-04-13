use std::collections::HashMap;

use crate::message::{EndpointStatus, QueryResult};

pub trait WaitingPool {
    fn submit(&mut self, result: QueryResult) -> Option<QueryResult>;
}

pub struct HashPool {
    pool: HashMap<u32, EndpointStatus>,
}

impl WaitingPool for HashPool {
    fn submit(&mut self, result: QueryResult) -> Option<QueryResult> {
        if result.status.is_success() {
            self.pool.remove(&result.endpoint.id);
            return None;
        }

        if let Some(status) = self.pool.get_mut(&result.endpoint.id) {
            status.results.push(result.clone());
            if status.results.len() >= status.endpoint.retry_count as usize {
                self.pool.remove(&result.endpoint.id);
            }
            Some(result.clone())
        } else {
            let status = EndpointStatus {
                endpoint: result.endpoint.clone(),
                results: vec![result.clone()],
                state: crate::message::EndpointState::Pending,
            };
            self.pool.insert(result.endpoint.id, status);
            Some(result.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Endpoint, EndpointState};
    use reqwest::StatusCode;
    use std::time::Duration;

    #[test]
    fn test_submit_success() {
        let mut hash_pool = HashPool {
            pool: HashMap::new(),
        };

        let endpoint = Endpoint {
            id: 1,
            url: "https://example.com".to_string(),
            retry_delay: Duration::from_secs(1),
            retry_count: 3,
            timeout: Duration::from_secs(5),
        };

        let result = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::OK,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        // Test submission of successful status
        let submit_result = hash_pool.submit(result);

        // Verify that successful status doesn't return any result
        assert!(submit_result.is_none());
        // Verify that successful requests are not added to the waiting pool
        assert!(hash_pool.pool.is_empty());
    }

    #[test]
    fn test_submit_failure() {
        let mut hash_pool = HashPool {
            pool: HashMap::new(),
        };

        let endpoint = Endpoint {
            id: 2,
            url: "https://example.com".to_string(),
            retry_delay: Duration::from_secs(1),
            retry_count: 3,
            timeout: Duration::from_secs(5),
        };

        let result = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        let submit_result = hash_pool.submit(result.clone());

        // Verify that failed status returns a result
        assert!(submit_result.is_some());
        let returned_result = submit_result.unwrap();
        assert_eq!(returned_result.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(returned_result.endpoint.id, endpoint.id);

        // Verify the state of the pool
        assert_eq!(hash_pool.pool.len(), 1);
        assert!(hash_pool.pool.contains_key(&endpoint.id));

        let status = hash_pool.pool.get(&endpoint.id).unwrap();
        assert_eq!(status.results.len(), 1);
        assert_eq!(status.results[0].status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(status.state, EndpointState::Pending);
    }

    #[test]
    fn test_submit_multiple_failures() {
        let mut hash_pool = HashPool {
            pool: HashMap::new(),
        };

        let endpoint = Endpoint {
            id: 3,
            url: "https://example.com".to_string(),
            retry_delay: Duration::from_secs(1),
            retry_count: 3,
            timeout: Duration::from_secs(5),
        };

        let result1 = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::BAD_GATEWAY,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        let result2 = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::SERVICE_UNAVAILABLE,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(150),
        };

        let submit_result1 = hash_pool.submit(result1);
        assert!(submit_result1.is_some());

        let submit_result2 = hash_pool.submit(result2);
        assert!(submit_result2.is_some());

        // Verify that multiple failed results are correctly accumulated
        assert_eq!(hash_pool.pool.len(), 1);
        let status = hash_pool.pool.get(&endpoint.id).unwrap();
        assert_eq!(status.results.len(), 2);
        assert_eq!(status.results[0].status, StatusCode::BAD_GATEWAY);
        assert_eq!(status.results[1].status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_submit_failure_then_success() {
        let mut pool = HashPool {
            pool: HashMap::new(),
        };

        let endpoint = Endpoint {
            id: 4,
            url: "https://example.com".to_string(),
            retry_delay: Duration::from_secs(1),
            retry_count: 3,
            timeout: Duration::from_secs(5),
        };

        let result_fail = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::NOT_FOUND,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        let result_success = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::OK,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        let submit_fail_result = pool.submit(result_fail);
        assert!(submit_fail_result.is_some());
        assert_eq!(pool.pool.len(), 1);

        let submit_success_result = pool.submit(result_success);
        assert!(submit_success_result.is_none());
        assert!(pool.pool.is_empty());
    }

    #[test]
    fn test_submit_max_retries_reached() {
        let mut pool = HashPool {
            pool: HashMap::new(),
        };

        let endpoint = Endpoint {
            id: 5,
            url: "https://example.com".to_string(),
            retry_delay: Duration::from_secs(1),
            retry_count: 2, // Set maximum retry count to 2
            timeout: Duration::from_secs(5),
        };

        let result1 = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::BAD_REQUEST,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(100),
        };

        let result2 = QueryResult {
            endpoint: endpoint.clone(),
            status: StatusCode::BAD_REQUEST,
            timestamp: chrono::Utc::now(),
            duration: tokio::time::Duration::from_millis(150),
        };

        // Submit the first failed result
        let submit_result1 = pool.submit(result1);
        assert!(submit_result1.is_some());
        assert_eq!(pool.pool.len(), 1);

        // Submit the second failed result, should reach maximum retry count
        let submit_result2 = pool.submit(result2);
        assert!(submit_result2.is_some());

        // Verify that the endpoint is removed from the pool after reaching maximum retry count
        assert!(pool.pool.is_empty());
    }
}
