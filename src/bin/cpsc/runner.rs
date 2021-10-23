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
    dirs::current_dir,
    git::{DynGit, GitCli, GitRepoKind, GitRepoTrait},
    repo_db::{NewOverlayOptions, NewStandaloneOptions, RepoDb, RepoEntry},
};
use crate::cli::{
    Cli, CliNewRepoName, CliRepoKind, ListFormat, OverlaySubcommand, RepoSpec, StandaloneSubcommand,
};
use anyhow::{anyhow, bail, Context};
use format::lazy_format;
use lifetime::{IntoStatic, ToBorrowed};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    fmt::{self, Debug, Display, Formatter},
    path::{Path, PathBuf},
    process::ExitStatus,
    str::FromStr,
};
use strum::IntoEnumIterator;

mod dirs;
pub mod git;
mod repo_db;

pub(crate) use self::{dirs::Directories, repo_db::RepoName};

#[derive(Debug)]
pub struct Runner {
    dirs: Directories,
    git: DynGit,
    repos: RepoDb,
}

impl RepoSpec {
    fn matches(&self, (_repo_name, repo): (RepoName<'_>, RepoEntry<'_>)) -> bool {
        match self {
            Self::All => true,
            &Self::Kind(kind) => repo.kind() == kind,
        }
    }
}

impl From<CliRepoKind> for GitRepoKind {
    fn from(kind: CliRepoKind) -> Self {
        match kind {
            CliRepoKind::Overlay => Self::Bare,
            CliRepoKind::Standalone => Self::Normal,
        }
    }
}

impl CliNewRepoName {
    fn unwrap_or_base_name(self, path: &Path) -> anyhow::Result<RepoName<'static>> {
        self.into_opt().map(Ok).unwrap_or_else(move || {
            let path_buf = if path.is_relative() {
                current_dir()?.join(path).clean()
            } else {
                path.to_owned()
            };
            // TODO: Do some
            path_buf
                .file_name()
                .with_context(|| anyhow!("no base name found for path {:?}", path))?
                .to_str()
                .context("base name is not UTF-8")
                .and_then(|base_name| Ok(RepoName::from_str(base_name)?))
                .context("base name for provided directory is not a valid repo name")
        })
    }
}

impl Runner {
    pub(crate) fn init(dirs: Directories) -> anyhow::Result<Self> {
        Ok(Runner {
            repos: RepoDb::new(&dirs)?,
            dirs,
            git: DynGit::Cli(GitCli),
        })
    }

    pub(crate) fn run(&mut self, cli_args: Cli) -> anyhow::Result<()> {
        let log_registered = |name, repo: RepoEntry<'_>| {
            log::info!("registered {:?} as {}", name, repo.short_desc());
        };
        match cli_args {
            Cli::Starter(_subcmd) => {
                bail!("`starter` commands are not implemented yet, stay tuned!")
            }
            Cli::Standalone(subcmd) => match subcmd {
                StandaloneSubcommand::Init { path, name } => {
                    let Self { dirs, git, repos } = self;
                    let path = path.map(Ok).unwrap_or_else(current_dir)?;
                    let name = name.unwrap_or_base_name(&path)?;
                    let (name, repo) = repos.new_standalone(
                        dirs,
                        git,
                        name,
                        path.into(),
                        None,
                        NewStandaloneOptions::Init,
                    )?;
                    log_registered(name, repo);
                    Ok(())
                }
                StandaloneSubcommand::Clone { name, path, source } => {
                    let Self { dirs, git, repos } = self;
                    let path = path.map(Ok).unwrap_or_else(|| -> anyhow::Result<_> {
                        let mut cwd = current_dir()?;
                        cwd.push::<&Path>(todo!(
                            "still haven't implemented getting a base name from the repo source"
                        ));
                        Ok(cwd)
                    })?;
                    let name = name.unwrap_or_base_name(&path)?;

                    let (name, repo) = repos.new_standalone(
                        dirs,
                        git,
                        name,
                        path.into(),
                        None,
                        NewStandaloneOptions::Clone { source },
                    )?;
                    log_registered(name, repo);
                    Ok(())
                }
                StandaloneSubcommand::Register { path, name } => {
                    let Self { repos, dirs, git } = self;

                    let path = path.map(Ok).unwrap_or_else(current_dir)?;
                    let name = name.unwrap_or_base_name(&path)?;

                    let (name, repo) = repos.new_standalone(
                        dirs,
                        git,
                        name,
                        path.into(),
                        None,
                        NewStandaloneOptions::Register,
                    )?;
                    log_registered(name, repo);
                    Ok(())
                }
                StandaloneSubcommand::Deregister { repo, name } => {
                    let Self {
                        repos,
                        git: _,
                        dirs,
                    } = self;

                    // TODO: ensure `repo` is after `--name` for forwards compatibility
                    let name = if name {
                        repo.context("`--name` was specified without a value")?
                            .to_str()
                            .context("name was not UTF-8")?
                            .parse::<RepoName<'static>>()?
                    } else {
                        let path = repo.map(Ok).unwrap_or_else(current_dir)?;
                        let (name, _repo) = repos.get_by_path(dirs, &path)?;
                        name.into_static()
                    };

                    let repo = repos.deregister_standalone(name.to_borrowed())?;
                    log::info!(
                        "deregistered {}; your files have been left intact",
                        repo.short_desc()
                    );
                    Ok(())
                }
            },
            Cli::Overlay(subcmd) => match subcmd {
                OverlaySubcommand::Init { name } => {
                    let Self { dirs, git, repos } = self;
                    let (name, repo) =
                        repos.new_overlay(dirs, git, name, NewOverlayOptions::Init)?;
                    log_registered(name, repo);
                    Ok(())
                }
                OverlaySubcommand::Clone {
                    name,
                    no_checkout,
                    source,
                } => {
                    let Self { dirs, git, repos } = self;
                    let name = name.into_opt().map(Ok).unwrap_or_else(|| -> anyhow::Result<_> {
                        todo!("still haven't implemented getting a base name from the repo source")
                    })?;
                    let (name, repo) = repos.new_overlay(
                        dirs,
                        git,
                        name,
                        NewOverlayOptions::Clone {
                            source,
                            no_checkout,
                        },
                    )?;
                    log_registered(name, repo);
                    Ok(())
                }
                OverlaySubcommand::RemoveBareRepo { name } => {
                    let Self {
                        dirs,
                        git: _,
                        repos,
                    } = self;
                    repos.remove_overlay_bare_repo(dirs, name.to_borrowed())?;
                    log::info!("removed bare Git repo for {:?}; your work tree files have been left intact", name);
                    Ok(())
                }
            },
            Cli::Run {
                repo_name,
                cd_root,
                cmd_and_args,
            } => {
                let Self { dirs, git, repos } = self;

                let mut cmd = cmd_and_args.to_std()?;

                let repo = repos
                    .get_by_name(repo_name.to_borrowed())
                    .with_context(|| {
                        anyhow!(
                            concat!(
                                "no repo configured with the name {:?} -- do you need to `",
                                env!("CARGO_BIN_NAME"),
                                " repo add`?",
                            ),
                            repo_name,
                        )
                    })?;

                let repo = {
                    if cd_root {
                        cmd.current_dir(repo.work_tree_path(dirs)?);
                    }
                    repo.open(git, dirs, repo_name)?
                };

                let cmd_status = repo.run_cmd(cmd, |mut cmd| {
                    log::debug!("running command {:?}", cmd);
                    cmd.status().context("failed to spawn command")
                })?;

                let _our_exit_code = match cmd_status.code() {
                    Some(code) => {
                        let display_exit_code =
                            lazy_format!(|f| { write!(f, "command returned exit code {}", code) });
                        if code == 0 {
                            log::debug!("{}", display_exit_code);
                        } else {
                            log::warn!("{}", display_exit_code);
                        }
                        code
                    }
                    None => {
                        log::warn!("command was terminated by a signal");
                        201 // TODO: actually design error codes for this command
                    }
                };

                // TODO: Return with exit code

                Ok(())
            }
            // TODO: This `allow` is necessary, but `clippy` throws a false positive. We need
            // to `collect` first in order to avoid borrowing `self` while iterating.
            #[allow(clippy::needless_collect)]
            Cli::ForEach {
                no_cd_root,
                cmd_and_args,
            } => {
                let mut err_happened = false;
                let names = self
                    .repos
                    .iter()
                    .map(|(name, repo)| (name.clone().into_static(), repo.short_desc().to_string()))
                    .collect::<Vec<_>>();
                names.into_iter().for_each(|(repo_name, repo_short_desc)| {
                    log::info!(
                        "running command against {:?} ({})",
                        repo_name,
                        repo_short_desc
                    );
                    match self
                        .run(Cli::Run {
                            repo_name: repo_name.clone(),
                            cd_root: !no_cd_root,
                            cmd_and_args: cmd_and_args.clone(),
                        })
                        .with_context(|| anyhow!("failed to run command for repo {:?}", repo_name))
                    {
                        Ok(()) => (),
                        Err(e) => {
                            err_happened = true;
                            log::error!("{}", e);
                        }
                    }
                });
                if err_happened {
                    Err(anyhow!(
                        "one or more errors occurred, see above output for more details"
                    ))
                } else {
                    Ok(())
                }
            }
            Cli::Remove { name } => {
                let Self { dirs, git, repos } = self;
                repos.try_remove_entire_repo(dirs, git, name)?;
                Ok(())
            }
            Cli::List { repo_spec, format } => {
                let Self {
                    dirs,
                    git: _, // TODO: diagnostics for broken stuff? :D
                    repos,
                } = self;
                let matching_repos_iter = || {
                    repos.iter().filter(|(name, repo)| {
                        repo_spec
                            .iter()
                            .all(|spec| spec.matches((name.to_borrowed(), repo.to_borrowed())))
                    })
                };
                match format {
                    ListFormat::Flat => {
                        matching_repos_iter().for_each(|(name, repo)| {
                            // TODO: Finalize this?
                            println!("{:?}: {}", name, repo.short_desc());
                        });
                    }
                    ListFormat::GroupByKind => {
                        CliRepoKind::iter().for_each(|repo_kind| {
                            // TODO: get casing right
                            println!("{:?}", repo_kind);
                            matching_repos_iter()
                                .filter(|(_name, repo)| repo.kind() == repo_kind)
                                .for_each(|(name, repo)| match repo_kind {
                                    CliRepoKind::Overlay => {
                                        println!("  {}", name);
                                    }
                                    CliRepoKind::Standalone => {
                                        println!(
                                            "  {}: {}",
                                            name,
                                            repo.path(dirs, name.to_borrowed()).unwrap().display()
                                        );
                                    }
                                })
                        });
                    }
                };
                Ok(())
            }
        }
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        let Self {
            repos,
            git: _,
            dirs,
        } = self;
        repos.flush(dirs)
    }
}

#[derive(
    Debug, Clone, Deserialize, Eq, IntoStatic, Ord, PartialEq, PartialOrd, ToBorrowed, Serialize,
)]
// TODO: research limits here
pub struct RemoteName<'a>(#[serde(borrow)] Cow<'a, str>);

impl Display for RemoteName<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self(inner) = self;
        Display::fmt(inner, f)
    }
}

fn canonicalize_path(path: &Path) -> anyhow::Result<PathBuf> {
    dunce::canonicalize(&path)
        .with_context(|| anyhow!("failed to canonicalize relative path {:?}", path))
}

fn cmd_failure_res(status: ExitStatus) -> anyhow::Result<()> {
    if let Some(err_msg) = cmd_failure_err(status) {
        Err(anyhow::Error::msg(err_msg))
    } else {
        Ok(())
    }
}

fn cmd_failure_err(status: ExitStatus) -> Option<Cow<'static, str>> {
    match status.code() {
        Some(0) => None,
        Some(code) => Some(format!("exited with exit status {}, see output above", code).into()),
        None => Some("command was terminated by a signal".into()),
    }
}
