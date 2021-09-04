use crate::{git::RepoSource, run_state::RepoName};
use clap::Clap;
use std::{ffi::OsString, path::PathBuf, process::Command, str::FromStr};
use strum::EnumIter;
use thiserror::Error as ThisError;

#[derive(Clap, Debug)]
#[clap(about, author)]
pub enum Cli {
    // Start {
    // starter: Option<PathBuf>,
    // }
    Show {
        #[clap(long, default_value = "all")]
        repo_spec: RepoSpec,
        #[clap(long, default_value = "name")]
        by: ShowBy,
        #[clap(long, conflicts_with = "by")]
        as_starter: bool,
    },
    #[clap(subcommand)]
    Repo(RepoSubcommand),
    // Sync {
    //     #[clap(long)]
    //     try_remove: bool,
    // },
    // Status,
}

#[derive(Clap, Debug)]
pub struct ShowSubcommand {}

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
pub enum ShowBy {
    Name,
    Kind,
}

impl Default for ShowBy {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, ThisError)]
#[error("invalid `by` spec; expected \"name\" or \"kind\", got {actual:?}")]
pub struct InvalidShowByError {
    actual: String,
}

impl FromStr for ShowBy {
    type Err = InvalidShowByError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "name" => Self::Name,
            "kind" => Self::Kind,
            actual => {
                return Err(InvalidShowByError {
                    actual: actual.to_string(),
                })
            }
        })
    }
}

#[derive(Clap, Debug)]
pub enum RepoSubcommand {
    // Init {
    //     name: RepoName<'static>,
    //     standalone: Option<PathBuf>,
    // },
    /// Clones a Git repository by cloning it from the specified `SOURCE`.
    ///
    /// If the target context already exists, this command makes no changes and exits with an
    /// error.
    #[clap(subcommand)]
    Add(RepoAddSubcommand),
    /// Runs a command
    Run {
        repo_name: RepoName<'static>,
        #[clap(long)]
        cd: bool,
        // #[clap(long)]
        // allow_standalone: bool,
        #[clap(flatten)]
        cmd_and_args: CommandAndArgs,
    },
    // ForEach {
    //     #[clap(flatten)]
    //     cmd_and_args: CommandAndArgs,
    // },
    Remove {
        repo_name: RepoName<'static>,
        #[clap(long)]
        no_delete: bool,
    },
    // Enter {
    //     repo_name: Option<RepoName<'static>>,
    //     #[clap(long)]
    //     cd: bool,
    // },
}

#[derive(Clap, Debug)]
pub enum RepoAddSubcommand {
    Overlay {
        /// The URL
        source: RepoSource<'static>,
        /// The alias by which this repo will be referred to when used later with this tool.
        ///
        /// TODO: discuss restrictions on the value provided heere
        /// TODO: make this optional, infer from `source`
        #[clap(long)]
        name: RepoName<'static>,
    },
    Standalone {
        path: PathBuf,
        /// The alias by which this repo will be referred to when used later with this tool.
        ///
        /// TODO: discuss restrictions on the value provided heere
        /// TODO: make this optional, infer from `source`
        #[clap(long)]
        name: RepoName<'static>,
        // /// The URL to use, when populating a path that does not exist yet.
        // #[clap(long)]
        // from_source: Option<RepoSource<'static>>,
        ///// The branch to check out from `source`, rather than the remote's `HEAD` branch.
        /////
        ///// No additional restrictions beyond Git's for branch names are imposed here.
        //#[clap(long)]
        //branch: Option<PathBuf>,
        //#[clap(long)]
        //remote: Option<PathBuf>,
    },
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

#[derive(Clap, Debug)]
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
