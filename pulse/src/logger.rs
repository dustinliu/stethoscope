use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

// Initializes the global logger using the `tracing` ecosystem.
// Configures logging based on the `RUST_LOG` environment variable, defaulting to `pulse=info`.
// Logs are written asynchronously to stdout, including line numbers.
// Returns a `WorkerGuard` which must be kept alive for the duration of the application
// to ensure all log messages are flushed.
pub fn init() -> tracing_appender::non_blocking::WorkerGuard {
    // Determine the logging level filter.
    // Try to read from the `RUST_LOG` environment variable.
    // If not set or invalid, default to showing `info` level logs for the `pulse` crate.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("pulse=info"));

    // Create a non-blocking writer that sends logs to stdout.
    // This prevents logging operations from blocking the main application threads.
    let (non_blocking_writer, guard) = tracing_appender::non_blocking(std::io::stdout());

    // Build the tracing subscriber registry.
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
