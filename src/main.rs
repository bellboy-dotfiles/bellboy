use self::{
    cli::Cli,
    run_state::{Directories, RunState},
};
use anyhow::Context;
use clap::Clap;

mod cli;
mod git;

mod run_state {
    use crate::{
        cli::{Cli, RepoAddSubcommand, RepoSubcommand},
        git::{Git, GitCli, GitRepoKind},
    };
    use anyhow::{anyhow, bail, Context, Result};
    use format::lazy_format;
    use lifetime::{IntoStatic, ToBorrowed};
    use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
    use std::{
        borrow::Cow,
        collections::BTreeMap,
        fmt::{self, Debug, Display, Formatter},
        fs::{self, OpenOptions},
        io::{BufReader, Read},
        path::{Path, PathBuf},
        str::FromStr,
    };
    use thiserror::Error as ThisError;
    use xdg::BaseDirectories;

    #[derive(Debug)]
    pub struct Directories {
        base_dirs: BaseDirectories,
    }

    impl Directories {
        pub fn new() -> anyhow::Result<Self> {
            // TODO: Use native config folders if they exist; warn that they're not portable.,
            let base_dirs = BaseDirectories::with_prefix(env!("CARGO_BIN_NAME"))
                .context("failed to detect XDG directories")?;
            Ok(Self { base_dirs })
        }

        fn local_repo_db_path(&self) -> anyhow::Result<PathBuf> {
            let Self { base_dirs } = self;
            base_dirs
                .place_data_file("local_repos.toml")
                .context("failed to place database file path")
        }

        fn global_repos_dir_path(&self) -> anyhow::Result<PathBuf> {
            let Self { base_dirs } = self;
            base_dirs
                .create_data_directory("global_repos")
                .context("failed to create global repos directory")
        }
    }

    #[derive(Debug)]
    pub struct RunState {
        dirs: Directories,
        git: Box<dyn Git>,
        repos: BTreeMap<RepoName<'static>, RepoEntry<'static>>,
        needs_persist: bool,
    }

    impl RunState {
        pub fn init(dirs: Directories) -> anyhow::Result<Self> {
            let repos = {
                let local_repos_db_path = dirs.local_repo_db_path()?;
                log::info!("local repos DB path: {}", local_repos_db_path.display());
                let db_toml = {
                    let mut buf = String::new();
                    let mut reader = BufReader::new(
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .open(&local_repos_db_path)
                            .with_context(|| {
                                anyhow!(
                                    "failed to open local repo file at {}",
                                    local_repos_db_path.display(),
                                )
                            })?,
                    );
                    reader.read_to_string(&mut buf).with_context(|| {
                        anyhow!(
                            "failed to read local repo contents at {}",
                            local_repos_db_path.display()
                        )
                    })?;
                    buf
                };

                let LocalRepoDatabase { local_repos } = if db_toml.trim().is_empty() {
                    LocalRepoDatabase::default()
                } else {
                    toml::from_str(&db_toml).with_context(|| {
                        anyhow!(
                            "failed to deserialize TOML from local repo file {}",
                            local_repos_db_path.display(),
                        )
                    })?
                };
                local_repos
                    .into_iter()
                    .map(|(name, LocalRepoEntry { path })| {
                        (
                            name.into_static(),
                            RepoEntry {
                                kind: RepoEntryKind::Local {
                                    repo_path: path.into_static(),
                                },
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            };

            let global_repos_dir_path = dirs.global_repos_dir_path()?;
            log::info!("global repos path: {}", global_repos_dir_path.display());

            // TODO: populate global repos by listing directory entries and checking if they're
            // really bare repos

            Ok(RunState {
                dirs,
                git: Box::new(GitCli),
                repos,
                needs_persist: false,
            })
        }

        pub fn run(&mut self, cli_args: Cli) -> anyhow::Result<()> {
            match cli_args {
                Cli::Repo(sub) => match sub {
                    RepoSubcommand::Add(sub) => {
                        let Self {
                            dirs,
                            git,
                            repos,
                            needs_persist,
                        } = self;

                        let (name, source, kind, repo_kind) = match sub {
                            RepoAddSubcommand::Local { path, name } => {
                                // TODO: Check that repo path isn't inside our data dir
                                (
                                    name,
                                    None,
                                    RepoEntryKind::Local {
                                        repo_path: path.into(),
                                    },
                                    GitRepoKind::Normal,
                                )
                            }
                            RepoAddSubcommand::Global { name, source } => (
                                name,
                                Some(source),
                                RepoEntryKind::Global {},
                                GitRepoKind::Bare,
                            ),
                        };

                        let path = kind.path(dirs, name.to_borrowed())?;
                        for (other_name, RepoEntry { kind }) in repos.iter() {
                            let names_match = &name == other_name;
                            let paths_match = kind.path(dirs, other_name.to_borrowed())? == path;
                            if names_match || paths_match {
                                if names_match && paths_match {
                                    bail!("repository is already added; did you accidentally repeat this command?")
                                } else {
                                    let repo_display = lazy_format!(|f| {
                                        match kind {
                                            RepoEntryKind::Local { repo_path } => {
                                                write!(f, "local repo at {}", repo_path.display())
                                            }
                                            RepoEntryKind::Global {} => {
                                                write!(f, "global repo")
                                            }
                                        }
                                    });
                                    bail!(
                                        "a repository with the name {:?} already exists as a {}",
                                        other_name,
                                        repo_display,
                                    );
                                }
                            }
                        }

                        if let Some(source) = source {
                            git.clone(path.as_ref(), source, repo_kind)
                                .context("failed to clone into Git")?;
                        } else {
                            // At least ensure that _something_ is there!
                            match git
                                .exists(path.as_ref(), repo_kind)
                                .context("failed trying to check if Git repo is present at path")?
                            {
                                Ok(()) => {
                                    log::info!(
                                        "validated that a {:?} repo exists at the provided path",
                                        repo_kind,
                                    );
                                }
                                Err(e) => bail!("Git repo check failed: {}", e),
                            }
                        }

                        repos.insert(name, RepoEntry { kind });
                        *needs_persist = true;
                        Ok(())
                    }
                },
            }
        }

        pub fn flush(&mut self) -> anyhow::Result<()> {
            let Self {
                dirs,
                git: _,
                repos,
                needs_persist,
            } = self;

            if !*needs_persist {
                return Ok(());
            }

            let local_repos = repos
                .iter()
                .filter_map(|(name, entry)| {
                    let RepoEntry { kind } = entry.to_borrowed();
                    match kind {
                        RepoEntryKind::Local { repo_path } => {
                            Some((name.to_borrowed(), LocalRepoEntry { path: repo_path }))
                        }
                        RepoEntryKind::Global { .. } => None,
                    }
                })
                .collect();

            let local_repos_db = LocalRepoDatabase { local_repos };

            let toml = toml::to_string(&local_repos_db)
                .expect("failed to serialize local repos DB as TOML");
            fs::write(dirs.local_repo_db_path()?, &toml).context("failed to write local repos DB")
        }
    }

    #[derive(Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
    struct LocalRepoDatabase<'a> {
        #[serde(borrow)]
        local_repos: BTreeMap<RepoName<'a>, LocalRepoEntry<'a>>,
    }

    #[derive(Debug, Deserialize, Eq, IntoStatic, Ord, PartialEq, PartialOrd, Serialize)]
    struct LocalRepoEntry<'a> {
        #[serde(borrow)]
        path: Cow<'a, Path>,
    }

    /// A name given to a repository
    #[derive(Clone, Eq, IntoStatic, Ord, PartialEq, PartialOrd, Serialize, ToBorrowed)]
    pub struct RepoName<'a>(#[serde(borrow)] Cow<'a, str>);

    impl Debug for RepoName<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            let Self(inner) = self;
            Debug::fmt(inner, f)
        }
    }

    impl RepoName<'_> {
        const SIZE_LIMIT: usize = 100;

        fn validate(name: &str) -> Result<(), InvalidRepoNameError> {
            // OPT: Could probably do some check specialized to upper bound on the size of 100
            // UTF-8 code points here.

            let mut chars = name.char_indices().enumerate();
            for (num, (idx, c)) in &mut chars {
                if num >= Self::SIZE_LIMIT {
                    return Err(InvalidRepoNameError::TooBig {
                        actual: num + chars.count(),
                    });
                }
                if !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-') {
                    return Err(InvalidRepoNameError::InvalidChar {
                        character: c,
                        at_byte: idx,
                    });
                }
            }
            Ok(())
        }

        pub fn as_single_path_segment(&self) -> &Path {
            let Self(inner) = self;
            Path::new(&*inner.as_ref())
        }
    }

    impl<'a> RepoName<'a> {
        pub fn new(name: Cow<'a, str>) -> Result<Self, InvalidRepoNameError> {
            Self::validate(&name).map(|()| Self(name))
        }
    }

    impl<'a, 'de: 'a> Deserialize<'de> for RepoName<'a> {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s = Cow::<'de, str>::deserialize(deserializer)?;
            Self::validate(&s)
                .map(|()| Self(s))
                .map_err(|e| D::Error::custom(e))
        }
    }

    #[derive(Debug, ThisError)]
    pub enum InvalidRepoNameError {
        #[error(
            "expected repo name to be less than {} characters; got {actual}",
            RepoName::SIZE_LIMIT
        )]
        TooBig { actual: usize },
        #[error(
            "expected repo name to only contain hyphens (\"-\"), periods (\".\"), or \
            alphanumeric characters; got {character:?} at {at_byte:?}"
        )]
        InvalidChar { character: char, at_byte: usize },
    }

    impl FromStr for RepoName<'static> {
        type Err = InvalidRepoNameError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            Self::validate(s).map(|()| Self(s.to_string().into()))
        }
    }

    #[derive(Debug, ToBorrowed)]
    pub struct RepoEntry<'a> {
        kind: RepoEntryKind<'a>,
    }

    impl RepoEntryKind<'_> {
        pub fn path(
            &self,
            dirs: &Directories,
            name: RepoName<'_>,
        ) -> anyhow::Result<Cow<'_, Path>> {
            Ok(match self {
                Self::Global {} => Self::global_path(dirs, name)?.into(),
                Self::Local { repo_path } => repo_path.to_borrowed(),
            })
        }

        fn global_path(dirs: &Directories, name: RepoName<'_>) -> anyhow::Result<PathBuf> {
            let mut path = dirs.global_repos_dir_path()?;
            path.push(name.as_single_path_segment());
            Ok(path)
        }
    }

    #[derive(Debug, ToBorrowed)]
    pub enum RepoEntryKind<'a> {
        /// A bare repository with a work tree in the user's home directory, set up by this tool.
        Global {},
        /// A whole (non-bare) Git repo located at `repo_path`.
        Local { repo_path: Cow<'a, Path> },
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
}

fn main() {
    env_logger::init();

    let command = Cli::parse();
    log::debug!("Parsed CLI args: {:?}", command);

    let res = (|| -> anyhow::Result<_> {
        let dirs = Directories::new()?;
        let mut rs = RunState::init(dirs).context("failed to initialize")?;
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
