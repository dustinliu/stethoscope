use crate::message::{Endpoint, EndpointStatus, QueryResult};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, broadcast, mpsc};

static HOLDER: OnceLock<Holder> = OnceLock::new();

struct Holder {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_tx: mpsc::Sender<QueryResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    status_tx: broadcast::Sender<EndpointStatus>,
    status_rx: broadcast::Receiver<EndpointStatus>,
}

impl Holder {
    fn new() -> Self {
        let (endpoint_tx, endpoint_rx) = mpsc::channel(100);
        let (result_tx, result_rx) = mpsc::channel(100);
        let (status_tx, status_rx) = broadcast::channel(100);

        let endpoint_rx = Arc::new(Mutex::new(endpoint_rx));
        let result_rx = Arc::new(Mutex::new(result_rx));

        Self {
            endpoint_tx,
            endpoint_rx,
            result_tx,
            result_rx,
            status_tx,
            status_rx,
        }
    }

    fn instance() -> &'static Holder {
        HOLDER.get_or_init(Self::new)
    }
}

pub struct Broker {
    endpoint_tx: mpsc::Sender<Endpoint>,
    endpoint_rx: Arc<Mutex<mpsc::Receiver<Endpoint>>>,
    result_tx: mpsc::Sender<QueryResult>,
    result_rx: Arc<Mutex<mpsc::Receiver<QueryResult>>>,
    status_tx: broadcast::Sender<EndpointStatus>,
    status_rx: broadcast::Receiver<EndpointStatus>,
}

impl Broker {
    pub fn instance() -> Self {
        let holder = Holder::instance();
        Self {
            endpoint_tx: holder.endpoint_tx.clone(),
            endpoint_rx: holder.endpoint_rx.clone(),
            result_tx: holder.result_tx.clone(),
            result_rx: holder.result_rx.clone(),
            status_tx: holder.status_tx.clone(),
            status_rx: holder.status_tx.subscribe(),
        }
    }
}
