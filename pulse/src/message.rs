/// Message module for the Pulse URL monitoring system
///
/// This module defines the data structures used for communication between
/// different components of the system, including requests, endpoints, and query results.
use reqwest::StatusCode;

/// Represents the configuration for a URL monitoring request
///
/// # Fields
/// * `url` - The URL to be monitored
/// * `timeout` - Maximum time to wait for the response
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub id: u32,
    pub url: String,
    pub failure_threshold: u8,
    pub timeout: tokio::time::Duration,
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (ID: {}, Threshold: {}, Timeout: {:?})",
            self.url, self.id, self.failure_threshold, self.timeout
        )
    }
}

/// Represents the result of a single endpoint monitoring attempt
///
/// # Fields
/// * `status` - HTTP status code of the response
/// * `timestamp` - When the query was executed
/// * `duration` - Duration from sending request to receiving response
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub endpoint: Endpoint,
    pub record: QueryRecord,
}

// Represents the detailed record of a single query attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRecord {
    // The HTTP status code returned by the endpoint.
    pub status: StatusCode,
    // The timestamp when the query was performed.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    // The duration the query took to complete.
    pub duration: tokio::time::Duration,
}

impl std::fmt::Display for QueryRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [{}] ({}ms)",
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.status,
            self.duration.as_millis()
        )
    }
}

// Represents the history of recent query attempts for a specific endpoint,
// particularly focusing on failures.
#[derive(Debug, Clone)]
pub struct EndpointHistory {
    // The endpoint configuration associated with this history.
    pub endpoint: Endpoint,
    // A vector of recent query records, typically storing consecutive failures.
    pub events: Vec<QueryRecord>,
}

impl std::fmt::Display for EndpointHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Endpoint checked failed: {}", self.endpoint)?;
        writeln!(f, "Events count: {}", self.events.len())?;

        for event in self.events.iter() {
            writeln!(f, "{}", event)?;
        }

        Ok(())
    }
}
