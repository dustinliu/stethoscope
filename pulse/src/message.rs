/// Message module for the Pulse URL monitoring system
///
/// This module defines the data structures used for communication between
/// different components of the system, including requests, endpoints, and query results.
use reqwest::StatusCode;

/// Represents the configuration for a URL monitoring request
///
/// # Fields
/// * `url` - The URL to be monitored
/// * `retry_delay` - Duration to wait before retrying a failed request
/// * `retry_count` - Maximum number of retry attempts for a failed request
/// * `timeout` - Maximum time to wait for the response
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub id: u32,
    pub url: String,
    pub retry_delay: tokio::time::Duration,
    pub retry_count: u8,
    pub timeout: tokio::time::Duration,
}

/// Represents the result of a single endpoint monitoring attempt
///
/// # Fields
/// * `status` - HTTP status code of the response
/// * `timestamp` - When the query was executed
/// * `duration` - Duration from sending request to receiving response
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub endpoint: Endpoint,
    pub status: StatusCode,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration: tokio::time::Duration,
}

#[derive(Debug, Clone)]
pub struct EndpointHistory {
    pub endpoint: Endpoint,
    pub results: Vec<QueryResult>,
}
