pub fn init() {
    // let filter = EnvFilter::builder()
    //     .with_default_directive(LevelFilter::INFO.into())
    //     .from_env()
    //     .unwrap()
    //     .add_directive("pulse=trace".parse().unwrap());

    tracing_subscriber::fmt()
        // .with_max_level(config::instance().log_level)
        .init();
}
