use clap::Parser;

use crate::app::App;

pub mod app;
pub mod event;
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

    #[arg(long, default_value_t = 2000)]
    pub update_interval: usize,

    #[arg(long, default_value_t = 100)]
    pub update_timeout: u32,

    #[arg(long, default_value_t = 1)]
    pub update_retries: u32,

    #[arg(long, default_value_t = 200)]
    pub discovery_timeout: u32,

    #[arg(long, default_value_t = 3)]
    pub discovery_retries: u32,

    #[arg(long, default_value_t = false)]
    pub include_hcas: bool,

    #[arg(long)]
    pub scope_file: Option<String>,

    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}

fn main() -> color_eyre::Result<()> {
    let args = Args::parse();
    let _stderr_gag: Option<gag::Gag> = gag::Gag::stderr().ok();
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = App::new(args).run(terminal);
    ratatui::restore();

    result
}
