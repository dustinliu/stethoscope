use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt}; // Add back the import

pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    // Add return type
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pulse=info"));

    // Create a non-blocking writer that writes to stdout.
    let (non_blocking_writer, guard) = tracing_appender::non_blocking(std::io::stdout());

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_line_number(true)
                .with_writer(non_blocking_writer),
        )
        .with(filter)
        .init();

    guard // Return the guard
}
