use clap::Parser;

use crate::app::App;

pub mod app;
pub mod event;
pub mod ui;
pub mod services;
pub mod scope;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long)]
    pub hca: String,
    
    #[arg(long, default_value_t = 0)]
    pub pkey: u32,

    #[arg(long, default_value_t = 16)]
    pub threads: usize,

    #[arg(long, default_value = "rsmad")]
    pub service_type: String,

    #[arg(long, default_value_t = 2)]
    pub update_interval: usize,

    #[arg(long, default_value_t = 1000)]
    pub timeout: u32,

    #[arg(long, default_value_t = 3)]
    pub retries: u32,

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
