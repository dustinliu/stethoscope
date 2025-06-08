use crate::message::Endpoint;
use rand::Rng;
use std::time::Duration;

pub struct EndpointProvider {}

impl EndpointProvider {
    pub fn new() -> Self {
        Self {}
    }

    pub fn get_endpoints(&self) -> Vec<Endpoint> {
        let mut rng = rand::rng();
        (0..=100)
            .map(|i| {
                let delay = rng.random_range(50..=200) as f64 / 1000.0; // Random delay between 50ms to 200ms
                Endpoint {
                    id: i,
                    url: format!("http://httpbin.org/delay/{}", delay),
                    timeout: Duration::from_secs(5),
                    failure_threshold: 3,
                }
            })
            .collect()
    }
}
