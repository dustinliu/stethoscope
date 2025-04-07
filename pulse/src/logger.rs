use env_logger::Builder;
use log::LevelFilter;

pub fn init_logger() {
    println!("Initializing logger");
    let mut builder = Builder::new();
    builder
        .filter_module("pulse", LevelFilter::Debug)
        .parse_default_env()
        .try_init()
        .expect("Failed to initialize logger");
}
