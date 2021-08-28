use crate::{git::RepoSource, run_state::RepoName};
use anyhow::anyhow;
use clap::Clap;
use std::{path::PathBuf, str::FromStr};

#[derive(Clap, Debug)]
#[clap(about, author)]
pub enum Cli {
    // Start {
    // starter: Option<PathBuf>,
    // }
    // Show {
    //     #[clap(long)]
    //     as_starter
    // },
    #[clap(subcommand)]
    Repo(RepoSubcommand),
    // Sync {
    //     #[clap(long)]
    //     try_remove: bool,
    // },
    // Status,
}

#[derive(Clap, Debug)]
pub enum RepoSubcommand {
    // Init {
    //     name: RepoName<'static>,
    //     local_path: Option<PathBuf>,
    // },
    /// Clones a Git repository by cloning it from the specified `SOURCE`.
    ///
    /// If the target context already exists, this command makes no changes and exits with an
    /// error.
    #[clap(subcommand)]
    Add(RepoAddSubcommand),
    // Run {
    //     repo_name: RepoName<'static>,
    //     #[clap(flatten)]
    //     cmd_args: CommandAndArgs,
    // },
    // ForEach {
    //     cmd_args: CommandAndArgs,
    // },
    // Remove {
    //     repo_name: RepoName<'static>,
    //     #[clap(long)]
    //     no_delete: bool,
    // },
    // Enter {
    //     repo_name: Option<RepoName<'static>>,
    // },
}

#[derive(Clap, Debug)]
pub enum RepoAddSubcommand {
    Global {
        /// The URL
        source: RepoSource<'static>,
        /// The alias by which this repo will be referred to when used later with this tool.
        ///
        /// TODO: discuss restrictions on the value provided heere
        /// TODO: make this optional, infer from `source`
        #[clap(long)]
        name: RepoName<'static>,
    },
    Local {
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

#[derive(Clone, Debug)]
pub enum CliRepoKind {
    Local,
    Global,
}

impl FromStr for CliRepoKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // TODO: How to make better diagnostics helping people with bad values?
            "local" => Ok(Self::Local),
            "global" => Ok(Self::Global),
            s => Err(anyhow!("unrecognized repo kind {:?}", s)),
        }
    }
}

// #[derive(Clap, Debug)]
// pub struct CommandAndArgs {
//     cmd: OsString,
//     args: Vec<OsString>,
// }
