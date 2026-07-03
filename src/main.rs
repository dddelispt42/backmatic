use backmatic::{Backmatic, CliArgs, Result};
use clap::Parser;

fn main() -> Result<()> {
    let cli = CliArgs::parse();
    let app = Backmatic::from_cli(cli)?;
    let code = app.run()?;
    std::process::exit(code);
}
