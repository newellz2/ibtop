use clap::Parser;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::app::App;

pub mod app;
pub mod event;
pub mod logging;
pub mod scope;
pub mod services;
pub mod ui;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long)]
    pub hca: String,

    #[arg(long, default_value_t = 0)]
    pub pkey: u32,

    #[arg(long, default_value_t = 16)]
    pub threads: usize,

    #[arg(long, default_value = "ibmad")]
    pub service_type: String,

    #[arg(long, default_value_t = 2)]
    pub update_interval: usize,

    #[arg(long, default_value_t = 250)]
    pub timeout: u32,

    #[arg(long, default_value_t = 2)]
    pub retries: u32,

    #[arg(long, default_value_t = false)]
    pub include_hcas: bool,

    #[arg(long)]
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
