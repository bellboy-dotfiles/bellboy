use self::{
    cli::Cli,
    run_state::{Directories, RunState}, // TODO: rename to `runner`?
};
use anyhow::Context;
use clap::Clap;

mod cli;
mod git;

mod run_state {
    use crate::{
        cli::{Cli, CliRepoKind, RepoAddSubcommand, RepoSpec, RepoSubcommand, ShowBy},
        git::{DynGit, DynGitRepo, GitCli, GitRepoKind, GitRepoTrait, GitTrait, OpenRepoOptions},
    };
    use anyhow::{anyhow, bail, Context, Result};
    use format::lazy_format;
    use lifetime::{IntoStatic, ToBorrowed};
    use remove_dir_all::remove_dir_all;
    use same_file::is_same_file;
    use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
    use std::{
        borrow::Cow,
        collections::BTreeMap,
        env::{current_dir, set_current_dir},
        fmt::{self, Debug, Display, Formatter},
        fs::{self, remove_file, OpenOptions},
        io::{BufReader, Read},
        path::{Path, PathBuf},
        str::FromStr,
    };
    use strum::IntoEnumIterator;
    use thiserror::Error as ThisError;
    use xdg::BaseDirectories;

    #[derive(Debug)]
    pub struct Directories {
        base_dirs: BaseDirectories,
    }

    impl Directories {
        pub fn new() -> anyhow::Result<Self> {
            // TODO: Use native config folders if they exist; warn that they're not portable.,
            let base_dirs = BaseDirectories::with_prefix(env!("CARGO_PKG_NAME"))
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

        fn home_dir_path(&self) -> anyhow::Result<PathBuf> {
            dirs::home_dir().context("user home directory not found")
        }
    }

    #[derive(Debug)]
    pub struct RunState {
        dirs: Directories,
        git: DynGit,
        repos: BTreeMap<RepoName<'static>, RepoEntry<'static>>,
        needs_persist: bool,
    }

    impl RepoSpec {
        fn matches(&self, (_repo_name, repo): (&RepoName<'_>, &RepoEntry)) -> bool {
            match self {
                Self::All => true,
                &Self::Kind(kind) => repo.kind() == kind,
            }
        }
    }

    impl RunState {
        pub fn init(dirs: Directories) -> anyhow::Result<Self> {
            let mut repos = {
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
                    // TODO: Validate duplicate entry handling.
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
            match global_repos_dir_path.read_dir().with_context(|| {
                anyhow!(
                    "failed to read global repo dirs from {}",
                    global_repos_dir_path.display(),
                )
            }) {
                Ok(entries) => {
                    entries.filter_map(|ent| {
                        (|| -> anyhow::Result<_> {
                            let ent = ent.with_context(|| anyhow!("failed to read a dir entry in global repo path"))?;

                            let file_name = ent.file_name();
                            let file_name = file_name.to_str().context("file name is not convertible to UTF-8")
                                .and_then(|finm| -> Result<RepoName<'static>> {
                                    finm.parse().map_err(anyhow::Error::new)
                                })
                                .with_context(|| anyhow!("file name {:?} is not a valid repo name", file_name))?;

                            if !ent.path().is_dir() {
                                log::warn!(
                                    "skipping global repo dir item {:?}, which does not appear to be a directory",
                                    file_name,
                                );
                                return Ok(None);
                            }

                            Ok(Some(file_name))
                        })().transpose()
                    }).try_for_each(|ent| {
                        match ent {
                            Ok(repo_name) => {
                                let repo = RepoEntry { kind: RepoEntryKind::Global {} };
                                log::trace!("found global repo {:?}", repo_name);
                                if let Some(first_repo) = repos.get(&repo_name) {
                                    bail!(
                                        "repo name conflict: repo name {:?} found as both:\n1. {}\n2. {}",
                                        repo_name,
                                        first_repo.short_desc(),
                                        repo.short_desc(),
                                    );
                                }
                                repos.insert(repo_name, repo);
                            }
                            Err(e) => log::warn!("{}", e),
                        };
                        Ok(())
                    })?;
                }
                Err(e) => log::warn!("{}", e),
            }

            Ok(RunState {
                dirs,
                git: DynGit::Cli(GitCli),
                repos,
                needs_persist: false,
            })
        }

        pub fn run(&mut self, cli_args: Cli) -> anyhow::Result<()> {
            match cli_args {
                Cli::Repo(sub) => {
                    match sub {
                        RepoSubcommand::Add(sub) => {
                            let Self {
                                dirs,
                                git,
                                repos,
                                needs_persist,
                            } = self;

                            let (name, source, kind, repo_kind) = match sub {
                                RepoAddSubcommand::Local { path, name } => {
                                    let path = if path.is_absolute() {
                                        path.into()
                                    } else {
                                        // Git doesn't understand UNC paths, which is what
                                        // `std::fs::canonicalize` converts paths to on Windows.
                                        // There's [reasons] for `std` to do this, but in our
                                        // context, this is undesirable. Try to avoid this using
                                        // `dunce` if at all possible.
                                        //
                                        // [reasons]: https://docs.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation?tabs=cmd
                                        dunce::canonicalize(&path)
                                            .with_context(|| {
                                                anyhow!(
                                                    "failed to canonicalize relative path {:?}",
                                                    path
                                                )
                                            })?
                                            .into()
                                    };
                                    // TODO: Check that repo path isn't inside our data dir
                                    (
                                        name,
                                        None,
                                        RepoEntryKind::Local { repo_path: path },
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
                            for (other_name, repo) in repos.iter() {
                                let names_match = &name == other_name;
                                let paths_match = {
                                    let other_repo_path =
                                        repo.path(dirs, other_name.to_borrowed())?;
                                    is_same_file(&path, &other_repo_path).unwrap_or_else(|e| {
                                        log::warn!(
                                            "failed to compare paths for equality: {:?}, {:?}: {}",
                                            path,
                                            other_repo_path,
                                            e,
                                        );
                                        false
                                    })
                                };
                                if names_match || paths_match {
                                    if names_match && paths_match {
                                        bail!("repository is already added; did you accidentally repeat this command?")
                                    } else {
                                        bail!(
                                            "a repository with the name {:?} already exists as a {}",
                                            other_name,
                                            repo.short_desc(),
                                        );
                                    }
                                }
                            }

                            if let Some(source) = source {
                                git.clone(path.as_ref(), source, repo_kind)
                                    .context("failed to clone into Git")?;
                            } else {
                                // At least ensure that _something_ is there!
                                match git.exists(path.as_ref(), repo_kind).context(
                                    "failed trying to check if Git repo is present at path",
                                )? {
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
                        RepoSubcommand::Run {
                            repo_name,
                            cd,
                            cmd_and_args,
                        } => {
                            let Self {
                                dirs,
                                git,
                                repos,
                                needs_persist: _,
                            } = self;

                            let mut cmd = cmd_and_args.to_std()?;

                            let repo = repos.get(&repo_name).with_context(|| {
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
                                if cd {
                                    cmd.current_dir(repo.work_tree_path(dirs)?);
                                }
                                repo.open(git, dirs, repo_name)?
                            };

                            let cmd_status = repo.run_cmd(cmd, |mut cmd| {
                                log::info!("running command {:?}", cmd);
                                let status = cmd.status().context("failed to spawn command");
                                log::debug!("returning from command");
                                status
                            })?;

                            let _our_exit_code = match cmd_status.code() {
                                Some(code) => {
                                    log::trace!("command returned exit code {}", code);
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
                        RepoSubcommand::Remove {
                            repo_name,
                            no_delete,
                        } => {
                            let Self {
                                dirs,
                                git,
                                repos,
                                needs_persist,
                            } = self;

                            let repo = repos.remove(&repo_name).with_context(|| {
                                anyhow!("no repo with the name {:?} is configured", repo_name)
                            })?;
                            *needs_persist = true;

                            // TODO: Seek confirmation. This is dangerous, yo.

                            // TODO: Check if there are any uncommitted files or branches, if so,
                            // seek confirmation.

                            if !no_delete {
                                match repo.kind() {
                                    CliRepoKind::Global => {
                                        let cwd = current_dir().context(
                                            "failed to copy current working directory path",
                                        )?;
                                        // Try to delete all files associated with this repo
                                        let repo = repo.open(git, dirs, repo_name.to_borrowed())?;
                                        match repo.list_files().context("failed to list files") {
                                            Ok(files) => {
                                                for file in files {
                                                    log::info!("removing {}", file.display());
                                                    match remove_file(&file) {
                                                        Ok(()) => (),
                                                        Err(e) => log::warn!(
                                                            "failed to remove {:?}: {}",
                                                            file,
                                                            e
                                                        ),
                                                    }
                                                }
                                            }
                                            Err(e) => log::warn!("{}", e),
                                        }
                                        set_current_dir(cwd).context("failed to switch back to original working directory path")?;
                                    }
                                    CliRepoKind::Local => (), // deleting the folder should suffice
                                }
                                let repo_path = repo.path(dirs, repo_name)?;
                                remove_dir_all(&repo_path).with_context(|| {
                                    anyhow!("failed to delete repo at {:?}", repo_path)
                                })?;
                            }

                            Ok(())
                        }
                    }
                }
                Cli::Show {
                    repo_spec,
                    by,
                    as_starter,
                } => {
                    if as_starter {
                        bail!("starters are not yet implemented; coming soon!")
                    } else {
                        let Self {
                            dirs,
                            git: _, // TODO: diagnostics for broken stuff? :D
                            repos,
                            needs_persist: _,
                        } = self;
                        match by {
                            ShowBy::Name => {
                                repos
                                    .iter()
                                    .filter(|repo| repo_spec.matches(*repo))
                                    .for_each(|(name, repo)| {
                                        // TODO: Finalize this?
                                        println!("{}: {}", name, repo.short_desc());
                                    });
                            }
                            ShowBy::Kind => {
                                CliRepoKind::iter().for_each(|repo_kind| {
                                    // TODO: get casing right
                                    println!("{:?}", repo_kind);
                                    repos
                                        .iter()
                                        .filter(|repo| repo_spec.matches(*repo))
                                        .filter(|(_name, repo)| repo.kind() == repo_kind)
                                        .for_each(|(name, repo)| match repo_kind {
                                            CliRepoKind::Global => {
                                                println!("  {}", name);
                                            }
                                            CliRepoKind::Local => {
                                                println!(
                                                    "  {}: {}",
                                                    name,
                                                    repo.path(dirs, name.to_borrowed())
                                                        .unwrap()
                                                        .display()
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

    impl Display for RepoName<'_> {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            let Self(inner) = self;
            Display::fmt(inner, f)
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

    impl RepoEntry<'_> {
        pub fn path(
            &self,
            dirs: &Directories,
            name: RepoName<'_>,
        ) -> anyhow::Result<Cow<'_, Path>> {
            let Self { kind } = self;
            kind.path(dirs, name)
        }

        pub fn work_tree_path(&self, dirs: &Directories) -> anyhow::Result<Cow<'_, Path>> {
            let Self { kind } = self;
            kind.work_tree_path(dirs)
        }

        pub fn short_desc(&self) -> impl Display + '_ {
            let Self { kind } = self;
            lazy_format!(move |f| {
                match kind {
                    RepoEntryKind::Local { repo_path } => {
                        write!(f, "local repo at {}", repo_path.display())
                    }
                    RepoEntryKind::Global {} => {
                        write!(f, "global repo")
                    }
                }
            })
        }

        pub fn open(
            &self,
            git: &DynGit,
            dirs: &Directories,
            name: RepoName<'_>,
        ) -> anyhow::Result<DynGitRepo> {
            let Self { kind } = self;

            let repo_path = kind.path(dirs, name.to_borrowed())?;
            let work_tree_path;
            let options = match kind {
                RepoEntryKind::Local { .. } => OpenRepoOptions::Normal {
                    work_tree_path: &*repo_path,
                },
                RepoEntryKind::Global { .. } => {
                    work_tree_path = kind.work_tree_path(dirs)?;
                    OpenRepoOptions::Bare {
                        repo_path: &*repo_path,
                        work_tree_path: &*work_tree_path,
                    }
                }
            };
            git.open_repo(options)
                .with_context(|| anyhow!("failed to open {:?} repo", name))
        }

        pub fn kind(&self) -> CliRepoKind {
            let Self { kind } = self;
            kind.kind()
        }
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

        pub fn work_tree_path(&self, dirs: &Directories) -> anyhow::Result<Cow<'_, Path>> {
            match self {
                Self::Global {} => dirs.home_dir_path().map(Into::into),
                Self::Local { repo_path } => Ok(repo_path.to_borrowed()),
            }
        }

        fn global_path(dirs: &Directories, name: RepoName<'_>) -> anyhow::Result<PathBuf> {
            let mut path = dirs.global_repos_dir_path()?;
            path.push(name.as_single_path_segment());
            Ok(path)
        }

        pub fn kind(&self) -> CliRepoKind {
            match self {
                Self::Local { .. } => CliRepoKind::Local,
                Self::Global { .. } => CliRepoKind::Global,
            }
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
