// Copyright 2021, Bellboy maintainers.
// This file is part of the [Bellboy project](https://github.com/bellboy-dotfiles/bellboy).
//
// Bellboy is free software: you can redistribute it and/or modify it under the terms of the GNU
// General Public License as published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// Bellboy is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without
// even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
// General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with Bellboy.  If not,
// see <https://www.gnu.org/licenses/>.
use anyhow::Context;
use directories::{BaseDirs, ProjectDirs};
use std::{
    env,
    fs::create_dir_all,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct Directories {
    base_dirs: BaseDirs,
    project_dirs: ProjectDirs,
}

impl Directories {
    pub(crate) fn new() -> anyhow::Result<Self> {
        // TODO: make this mockable
        let this = Self {
            base_dirs: BaseDirs::new().context("no home directory found for current user")?, // error message based on documented error cases for `BaseDirs::new`
            project_dirs: ProjectDirs::from(
                "", // TODO: Is this right?
                "bellboy-dotfiles",
                env!("CARGO_PKG_NAME"),
            )
            .unwrap(),
        };
        create_dir_all(
            this.overlay_repos_dir_path()
                .context("failed to get overlay repos directory path")?,
        )
        .context("failed to create overlay repos directory path")?;
        Ok(this)
    }

    pub(crate) fn home_dir_path(&self) -> anyhow::Result<PathBuf> {
        // TODO: Remove `Result`, return a reference
        Ok(self.base_dirs.home_dir().to_path_buf())
    }

    pub(crate) fn overlay_repos_dir_path(&self) -> anyhow::Result<PathBuf> {
        // TODO: Remove `Result`
        Ok(self.project_dirs.data_local_dir().join("overlay_repos/"))
    }

    pub(crate) fn standalone_repo_db_path(&self) -> anyhow::Result<PathBuf> {
        // TODO: Remove `Result`
        Ok(self
            .project_dirs
            .data_local_dir()
            .join("standalone_repos.toml"))
    }
}

pub(crate) fn current_dir() -> anyhow::Result<PathBuf> {
    env::current_dir().context("failed to get current working directory path")
}

pub(crate) fn set_current_dir(path: &Path) -> anyhow::Result<()> {
    env::set_current_dir(path).context("failed to set current working directory path")
}
