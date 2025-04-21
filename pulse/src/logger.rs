use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pulse=info"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
