use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::*;

pub fn init_logger() {
    let logging_mode = "debug";

    let indicatif_layer = tracing_indicatif::IndicatifLayer::new();

    let fmt_layer = fmt::layer()
        .pretty()
        .with_writer(indicatif_layer.get_stderr_writer())
        .with_thread_names(true)
        .with_ansi(true)
        .without_time()
        .with_filter(EnvFilter::new(logging_mode));

    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(format!("{logging_mode},tokio=trace,runtime=trace,hyper=info,tokio_postgres=info,bitcoincore_rpc=info")))
        .unwrap();

    let logger = tracing_subscriber::registry().with(filter_layer).with(fmt_layer).with(indicatif_layer);

    logger.init();
}
