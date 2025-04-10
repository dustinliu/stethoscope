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
    pub id: u64,
    pub url: String,
    pub retry_delay: std::time::Duration,
    pub retry_count: u8,
    pub timeout: std::time::Duration,
}

/// Represents an endpoint with its monitoring configuration and results
///
/// # Fields
/// * `request` - The endpoint configuration
/// * `result` - Vector of results from each monitoring attempt
#[derive(Debug, Clone)]
pub struct EndpointResponse {
    pub request: Endpoint,
    pub results: Vec<QueryResult>,
}

/// Represents the result of a single endpoint monitoring attempt
///
/// # Fields
/// * `status` - HTTP status code of the response
/// * `timestamp` - When the query was executed
/// * `duration` - Duration from sending request to receiving response
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub status: StatusCode,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration: std::time::Duration,
}
