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
use crate::runner::{git::RepoSource, RepoName};
use clap::Parser;
use std::{ffi::OsString, path::PathBuf, process::Command, str::FromStr};
use strum::EnumIter;
use thiserror::Error as ThisError;

#[derive(Debug, Parser)]
#[clap(about, author, version)]
pub(crate) enum Cli {
    /// Use a starter file to quickly import or export a configuration.
    ///
    /// TODO: There's lots of ambitions for starter files, but they're yet to be fully designed or
    /// implemented. Stay tuned!
    #[clap(subcommand)]
    Starter(StarterSubcommand),
    /// Control the lifecycle of a stand-alone repo entry.
    ///
    /// `standalone` repos are what people typically think of when they say "Git repo": a local
    /// copy of a Git repository.
    #[clap(subcommand)]
    Standalone(StandaloneSubcommand),
    /// Control the lifecycle of an overlay repo entry.
    ///
    /// `overlay` repos are bare repos rooted a home directory, configured such that overlapping
    /// work trees can coexist peacefully. They're handy for dotfiles that don't have their own
    /// dedicated folder.
    #[clap(subcommand)]
    Overlay(OverlaySubcommand),
    /// Invoke a command against a repo.
    ///
    /// Currently, this command sets the `GIT_DIR` and `GIT_WORK_TREE` variables for the invoked
    /// command. This behavior is not stable, and may be redesigned before 1.0.0.
    Run {
        repo_name: RepoName<'static>,
        #[clap(long)]
        cd_root: bool,
        // #[clap(long)]
        // allow_standalone: bool,
        #[clap(flatten)]
        cmd_and_args: CommandAndArgs,
    },
    /// Invoke a command against all repos.
    ///
    /// This command does the same as the `run`, except it (1) runs on all configured repos, and
    /// (2) by default, the working directory for each command invocation is set to the work tree
    /// root of the repo entry it's running against.
    ForEach {
        /// If set, uses the working directory of this tool's invocation, rather than the work tree
        /// root, for each repo entry command invocation.
        #[clap(long)]
        no_cd_root: bool,
        #[clap(flatten)]
        cmd_and_args: CommandAndArgs,
    },
    /// Remove a repo entry, attempting to remove all files associated with the repo's work tree.
    Remove {
        name: RepoName<'static>,
        // // TODO: `--allow-dirty` subcommand
        // allow_dirty: bool,
    },
    // // TODO: A crazy ambitious idea to use the user's auto-magically detected shell?
    // Preposterous. :)
    // Enter {
    //     repo_name: Option<RepoName<'static>>,
    //     #[clap(long)]
    //     cd: bool,
    // },
    /// List repo entries in the current configuration.
    ///
    /// TODO: document repo spec and format options.
    List {
        #[clap(default_value = "all")]
        repo_spec: Vec<RepoSpec>,
        #[clap(long, default_value = "flat")]
        format: ListFormat,
    },
    // // TODO: Might be nice to give a condensed presentation of files listed by `git status`?
    // Status,
}

#[derive(Debug, Parser)]
pub enum StarterSubcommand {
    /// Import a starter file from `PATH`.
    Import {
        path: PathBuf,
        /// If specified, attempt to interpret `PATH` as a relative path into the given Git repo
        /// source.
        git: RepoSource<'static>,
    },
    /// Export a starter file to `PATH`.
    Export { path: PathBuf },
}

#[derive(Debug, Parser)]
pub struct ListSubcommand {}

#[derive(Debug)]
pub enum RepoSpec {
    All,
    // Name(Regex),
    Kind(CliRepoKind),
}

impl Default for RepoSpec {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, ThisError)]
pub enum InvalidRepoSpecError {
    #[error(
        "{what:?} is not a recognized repo spec; expected \"all\" \
        or spec of the form \"<type>:<value>\""
    )]
    Unrecognized { what: String },
    #[error("{what:?} is not a recognized parameterized spec type")]
    UnrecognizedType { what: String },
    #[error("failed to parse `kind`")]
    ParseRepoKind { source: InvalidRepoKindError },
}

impl FromStr for RepoSpec {
    type Err = InvalidRepoSpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "all" => Self::All,
            s => {
                if let Some((type_, value)) = s.split_once(':') {
                    match type_ {
                        "kind" => Self::Kind(
                            value
                                .parse()
                                .map_err(|source| InvalidRepoSpecError::ParseRepoKind { source })?,
                        ),
                        s => {
                            return Err(InvalidRepoSpecError::UnrecognizedType {
                                what: s.to_string(),
                            });
                        }
                    }
                } else {
                    return Err(InvalidRepoSpecError::Unrecognized {
                        what: s.to_string(),
                    });
                }
            }
        })
    }
}

#[derive(Debug)]
pub enum ListFormat {
    Flat,
    GroupByKind,
}

impl Default for ListFormat {
    fn default() -> Self {
        Self::Flat
    }
}

#[derive(Debug, ThisError)]
#[error("invalid `by` spec; expected \"flat\", or \"group-by-kind\", but got {actual:?}")]
pub struct InvalidListFormatError {
    actual: String,
}

impl FromStr for ListFormat {
    type Err = InvalidListFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "flat" => Self::Flat,
            "group-by-kind" => Self::GroupByKind,
            actual => {
                return Err(InvalidListFormatError {
                    actual: actual.to_string(),
                })
            }
        })
    }
}

#[derive(Debug, Parser)]
pub enum StandaloneSubcommand {
    Init {
        path: Option<PathBuf>,
        #[clap(flatten)]
        name: CliNewRepoName,
    },
    /// Clone a Git repository from the specified `SOURCE`.
    ///
    /// If the target context already exists, this command makes no changes and exits with an
    /// error.
    Clone {
        /// The source path or URL of the repo to clone.
        source: RepoSource<'static>,
        path: Option<PathBuf>,
        #[clap(flatten)]
        name: CliNewRepoName,
    },
    /// Registers a standalone repo that already exists at `DIR`.
    Register {
        path: Option<PathBuf>,
        #[clap(flatten)]
        name: CliNewRepoName,
    },
    /// Deregister `REPO` without deleting files.
    ///
    /// This subcommand makes no attempt to remove local files; it only removes this tool's
    /// awareness of them. If you also wish to remove all files, you may instead prefer to use the
    /// top-level `remove` subcommand.
    Deregister {
        /// The repo to deregister. Interpreted as a path, unless `--name` is specified, in which
        /// case this is interpreted as a repo name.
        repo: Option<PathBuf>,
        #[clap(long)]
        name: bool,
    },
    // // TODO:
    // SetProjectDetails
}

#[derive(Debug, Parser)]
pub enum OverlaySubcommand {
    /// Initialize a new `overlay` repo.
    Init {
        /// The alias by which this repo will be referred to when used later with this tool, if you
        /// wish to override what would be inferred.
        ///
        /// TODO: discuss restrictions on the value provided heere
        name: RepoName<'static>,
    },
    /// Clone a Git repository from the specified `SOURCE`.
    ///
    /// If the target context already exists, this command makes no changes and exits with an
    /// error.
    Clone {
        /// The URL
        source: RepoSource<'static>,
        #[clap(flatten)]
        name: CliNewRepoName,
        /// Disables population of the work tree (user home directory) after cloning the bare repo.
        ///
        /// Useful for recreating your overlay repo after calling `remove-bare-repo`.
        #[clap(long)]
        no_checkout: bool,
    },
    /// Remove an `overlay` repo's Git files, leaving the worktree intact.
    ///
    /// This subcommand makes no attempt to remove the work tree files associated with the
    /// specified repo; it only removes this tool's awareness of them. If you also wish to remove
    /// all files, you may instead prefer to use the top-level `remove` subcommand.
    RemoveBareRepo { name: RepoName<'static> },
}

#[derive(Debug, Parser)]
pub struct CliExistingRepoName {
    /// A repo name previously added to this tool's configuration.
    pub name: RepoName<'static>,
}

#[derive(Debug, Parser)]
pub struct CliNewRepoName {
    /// The alias by which this repo will be referred to when used later with this tool, if you
    /// wish to override what would be inferred.
    ///
    /// TODO: discuss restrictions on the value provided heere
    #[clap(long)]
    name: Option<RepoName<'static>>,
}

impl CliNewRepoName {
    pub fn into_opt(self) -> Option<RepoName<'static>> {
        let Self { name } = self;
        name
    }
}

pub trait NewRepoNameContainer {
    type Output;
}

#[derive(Copy, Clone, Debug, EnumIter, Eq, PartialEq)]
pub enum CliRepoKind {
    Standalone,
    Overlay,
}

#[derive(Debug, ThisError)]
#[error("unrecognized repo kind {what:?}")]
pub struct InvalidRepoKindError {
    what: String,
}

impl FromStr for CliRepoKind {
    type Err = InvalidRepoKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // TODO: How to make better diagnostics helping people with bad values?
            "standalone" => Ok(Self::Standalone),
            "overlay" => Ok(Self::Overlay),
            s => Err(InvalidRepoKindError { what: s.to_owned() }),
        }
    }
}

#[derive(Parser, Clone, Debug)]
pub struct CommandAndArgs {
    #[clap(raw(true))]
    cmd_and_args: Vec<OsString>,
}

#[derive(Debug, ThisError)]
pub enum CommandError {
    #[error("command not specified")]
    CommandNotSpecified,
}

impl CommandAndArgs {
    pub fn to_std(&self) -> Result<Command, CommandError> {
        let Self { cmd_and_args } = self;
        let (cmd, args) = cmd_and_args
            .split_first()
            .ok_or(CommandError::CommandNotSpecified)?;
        let mut cmd = Command::new(cmd);
        cmd.args(args);
        Ok(cmd)
    }
}
