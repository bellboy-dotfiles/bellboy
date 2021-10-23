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
use anyhow::Context;
use std::{
    env,
    path::{Path, PathBuf},
};
use xdg::BaseDirectories;

#[derive(Debug)]
pub(crate) struct Directories {
    base_dirs: BaseDirectories,
}

impl Directories {
    pub(crate) fn new() -> anyhow::Result<Self> {
        // TODO: Use native config folders if they exist; warn that they're not portable.,
        let base_dirs = BaseDirectories::with_prefix(env!("CARGO_PKG_NAME"))
            .context("failed to detect XDG directories")?;
        Ok(Self { base_dirs })
    }

    pub(crate) fn home_dir_path(&self) -> anyhow::Result<PathBuf> {
        dirs::home_dir().context("user home directory not found")
    }

    pub(crate) fn overlay_repos_dir_path(&self) -> anyhow::Result<PathBuf> {
        let Self { base_dirs } = self;
        base_dirs
            .create_data_directory("overlay_repos")
            .context("failed to create overlay repos directory")
    }

    pub(crate) fn standalone_repo_db_path(&self) -> anyhow::Result<PathBuf> {
        let Self { base_dirs } = self;
        base_dirs
            .place_data_file("standalone_repos.toml")
            .context("failed to place database file path")
    }
}

pub(crate) fn current_dir() -> anyhow::Result<PathBuf> {
    env::current_dir().context("failed to get current working directory path")
}

pub(crate) fn set_current_dir(path: &Path) -> anyhow::Result<()> {
    env::set_current_dir(path).context("failed to set current working directory path")
}
