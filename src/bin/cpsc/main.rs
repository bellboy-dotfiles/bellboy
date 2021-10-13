// Copyright 2021, Capisco maintainers.
// This file is part of the [Capisco project](https://github.com/capisco-dotfiles/capisco).
//
// Capisco is free software: you can redistribute it and/or modify it under the terms of the GNU
// General Public License as published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// Capisco is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without
// even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
// General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with Capisco.  If not,
// see <https://www.gnu.org/licenses/>.
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
