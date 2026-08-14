pub mod config;

pub fn init_tracing(json_logs: bool) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if json_logs {
        tracing_subscriber::fmt()
            .json()
            .with_file(true)
            .with_line_number(true)
            .with_target(true)
            .with_env_filter(env_filter)
            .try_init()
            .ok();
    } else {
        tracing_subscriber::fmt()
            .with_file(true)
            .with_line_number(true)
            .with_target(true)
            .with_env_filter(env_filter)
            .try_init()
            .ok();
    }
}
