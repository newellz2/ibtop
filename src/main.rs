use clap::Parser;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::app::App;

pub mod app;
pub mod event;
pub mod logging;
pub mod scope;
pub mod services;
pub mod ui;

/// Every option can also be set via an `IBTOP_*` environment variable (shown
/// in `--help`); an explicit command-line flag wins over the environment.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, env = "IBTOP_HCA")]
    pub hca: String,

    /// P_Key table *index* (not a P_Key value) for PerfMgt queries.
    #[arg(long, alias = "pkey", env = "IBTOP_PKEY_INDEX", default_value_t = 0)]
    pub pkey_index: u16,

    /// Worker threads (each with its own UMAD fd) for counter queries.
    #[arg(long, env = "IBTOP_THREADS", default_value_t = 16)]
    pub threads: usize,

    #[arg(long, env = "IBTOP_SERVICE_TYPE", default_value = "ibmad")]
    pub service_type: String,

    #[arg(long, env = "IBTOP_UPDATE_INTERVAL", default_value_t = 2)]
    pub update_interval: usize,

    #[arg(long, env = "IBTOP_TIMEOUT", default_value_t = 250)]
    pub timeout: u32,

    #[arg(long, env = "IBTOP_RETRIES", default_value_t = 2)]
    pub retries: u32,

    #[arg(long, env = "IBTOP_INCLUDE_HCAS", default_value_t = false)]
    pub include_hcas: bool,

    /// Service level (0-15) for PerfMgt counter queries to switches and HCAs.
    #[arg(long, env = "IBTOP_SL", default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=15))]
    pub sl: u8,

    #[arg(long, env = "IBTOP_SCOPE_FILE")]
    pub scope_file: Option<String>,

    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    #[arg(long, default_value_t = false)]
    pub tracing: bool,
}

fn main() -> color_eyre::Result<()> {
    let args = Args::parse();
    if args.tracing {
        logging::initialize_logging()?;
    }
    let _stderr_gag: Option<gag::Gag> = gag::Gag::stderr().ok();
    color_eyre::install()?;
    let terminal = ratatui::init();
    // Ensure we restore the terminal even if the app panics (e.g. due to service thread issues).
    let result = catch_unwind(AssertUnwindSafe(|| App::new(args).run(terminal)));
    ratatui::restore();

    match result {
        Ok(r) => r,
        Err(panic_payload) => {
            eprintln!(
                "ibtop panicked (set RUST_BACKTRACE=1 for more detail). payload: {:?}",
                panic_payload
            );
            Err(color_eyre::eyre::eyre!("ibtop panicked"))
        }
    }
}
