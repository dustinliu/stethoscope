use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pulse=info"));

    let (non_blocking_writer, guard) = tracing_appender::non_blocking(std::io::stdout());

    tracing_subscriber::registry()
        .with(
            // Add a formatting layer.
            fmt::layer()
                // Include the source code line number in log messages.
                .with_line_number(true)
                // Use the non-blocking writer for output.
                .with_writer(non_blocking_writer),
        )
        // Apply the previously determined filter.
        .with(filter)
        // Set this configuration as the global default subscriber.
        .init();

    // Return the guard. It must be held onto to ensure logs are flushed when the application exits.
    guard
}
