use self::{
    cli::Cli,
    runner::{Directories, Runner}, // TODO: rename to `runner`?
};
use anyhow::Context;
use clap::Clap;

mod cli;
mod runner;

fn main() {
    colog::init();

    let command = Cli::parse();
    log::trace!("Parsed CLI args: {:?}", command);

    let res = (|| -> anyhow::Result<_> {
        let dirs = Directories::new()?;
        let mut rs = Runner::init(dirs).context("failed to initialize")?;
        rs.run(command)?;

        log::trace!("flushing data");
        rs.flush().context("failed to flush data")?;

        Ok(())
    })();
    match res {
        Ok(()) => (),
        Err(e) => log::error!("{:?}", e),
    }
}
