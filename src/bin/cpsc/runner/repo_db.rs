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
use crate::{
    cli::CliRepoKind,
    runner::{
        canonicalize_path,
        dirs::Directories,
        git::{DynGit, DynGitRepo, GitRepoTrait, GitTrait, OpenRepoOptions, RepoSource},
    },
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use format::lazy_format;
use lifetime::{IntoStatic, ToBorrowed};
use path_dsl::path;
use remove_dir_all::remove_dir_all;
use same_file::is_same_file;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt::{self, Debug, Display, Formatter},
    fs::{self, create_dir, remove_file, OpenOptions},
    io::{self, BufReader, Read},
    mem::transmute,
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error as ThisError;

#[derive(Debug)]
pub(super) struct RepoDb {
    repos: BTreeMap<RepoName<'static>, RepoEntry<'static>>,
    needs_persist: bool,
}

/// A name given to a repository
#[derive(Clone, Eq, IntoStatic, Ord, PartialEq, PartialOrd, Serialize, ToBorrowed)]
pub struct RepoName<'a>(#[serde(borrow)] Cow<'a, str>);

impl Deref for RepoName<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        let Self(c) = self;
        &*c
    }
}

impl Debug for RepoName<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self(inner) = self;
        Debug::fmt(inner, f)
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

#[derive(Debug, IntoStatic, ToBorrowed)]
pub struct RepoEntry<'a> {
    kind: RepoEntryKind<'a>,
}

impl<'a> RepoEntry<'a> {}

impl RepoEntry<'_> {
    pub(crate) fn path(
        &self,
        dirs: &Directories,
        name: RepoName<'_>,
    ) -> anyhow::Result<Cow<'_, Path>> {
        let Self { kind } = self;
        kind.path(dirs, name)
    }

    pub(crate) fn work_tree_path(&self, dirs: &Directories) -> anyhow::Result<Cow<'_, Path>> {
        let Self { kind } = self;
        kind.work_tree_path(dirs)
    }

    pub(crate) fn short_desc(&self) -> impl Display + '_ {
        let Self { kind } = self;
        lazy_format!(move |f| {
            match kind {
                RepoEntryKind::Standalone { app_info: _, path } => {
                    write!(f, "standalone repo at {}", path.display())
                }
                RepoEntryKind::Overlay {} => {
                    write!(f, "overlay repo")
                }
            }
        })
    }

    pub(crate) fn open(
        &self,
        git: &DynGit,
        dirs: &Directories,
        name: RepoName<'_>,
    ) -> anyhow::Result<DynGitRepo> {
        let Self { kind } = self;

        let repo_path = kind.path(dirs, name.to_borrowed())?;
        let work_tree_path;
        let options = match kind {
            RepoEntryKind::Standalone { .. } => OpenRepoOptions::Normal {
                work_tree_path: &*repo_path,
            },
            RepoEntryKind::Overlay { .. } => {
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

#[derive(Debug, IntoStatic, ToBorrowed)]
enum RepoEntryKind<'a> {
    /// A bare Git repository with a work tree in the user's home directory, set up by this tool.
    Overlay {},
    /// A whole (non-bare) Git repository located at `repo_path`.
    Standalone {
        path: Cow<'a, Path>,
        app_info: Option<AppInfo<'a>>,
    },
}

impl RepoEntryKind<'_> {
    pub fn path(&self, dirs: &Directories, name: RepoName<'_>) -> anyhow::Result<Cow<'_, Path>> {
        Ok(match self {
            Self::Overlay {} => Self::overlay_path(dirs, name)?.into(),
            Self::Standalone { app_info: _, path } => path.to_borrowed(),
        })
    }

    pub fn work_tree_path(&self, dirs: &Directories) -> anyhow::Result<Cow<'_, Path>> {
        match self {
            Self::Overlay {} => dirs.home_dir_path().map(Into::into),
            Self::Standalone { app_info: _, path } => Ok(path.to_borrowed()),
        }
    }

    fn overlay_path(dirs: &Directories, name: RepoName<'_>) -> anyhow::Result<PathBuf> {
        let mut path = dirs.overlay_repos_dir_path()?;
        path.push(name.as_single_path_segment());
        Ok(path)
    }

    pub fn kind(&self) -> CliRepoKind {
        match self {
            Self::Standalone { .. } => CliRepoKind::Standalone,
            Self::Overlay { .. } => CliRepoKind::Overlay,
        }
    }
}

impl RepoDb {
    pub fn new(dirs: &Directories) -> anyhow::Result<Self> {
        let mut repos = {
            StandaloneRepoDb::from_toml_on_disk(dirs)?
                .into_runner_repos()
                .collect::<BTreeMap<_, _>>()
        };

        let overlay_repos_dir_path = dirs.overlay_repos_dir_path()?;
        log::trace!("overlay repos path: {}", overlay_repos_dir_path.display());
        match overlay_repos_dir_path.read_dir().with_context(|| {
            anyhow!(
                "failed to read overlay repo dirs from {}",
                overlay_repos_dir_path.display(),
            )
        }) {
            Ok(entries) => {
                entries.filter_map(|ent| {
                        (|| -> anyhow::Result<_> {
                            let ent = ent.with_context(|| anyhow!("failed to read a dir entry in overlay repo path"))?;

                            let file_name = ent.file_name();
                            let file_name = file_name.to_str().context("file name is not convertible to UTF-8")
                                .and_then(|finm| -> Result<RepoName<'static>> {
                                    finm.parse().map_err(anyhow::Error::new)
                                })
                                .with_context(|| anyhow!("file name {:?} is not a valid repo name", file_name))?;

                            if !ent.path().is_dir() {
                                log::warn!(
                                    "skipping overlay repo dir item {:?}, which does not appear to be a directory",
                                    file_name,
                                );
                                return Ok(None);
                            }

                            Ok(Some(file_name))
                        })().transpose()
                    }).try_for_each(|ent| {
                        match ent {
                            Ok(repo_name) => {
                                let repo = RepoEntry { kind: RepoEntryKind::Overlay {} };
                                log::trace!("found overlay repo {:?}", repo_name);
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

        Ok(Self {
            repos,
            needs_persist: false,
        })
    }

    /// # Panics
    ///
    /// You should call [`Self::validate_no_add_conflicts`] first!
    fn insert(
        &mut self,
        name: RepoName<'static>,
        repo: RepoEntry<'static>,
    ) -> (RepoName<'_>, RepoEntry<'_>) {
        let Self {
            repos,
            needs_persist,
        } = self;
        assert!(repos.insert(name.clone(), repo).is_none());
        *needs_persist = true;

        let (name, repo) = repos.get_key_value(&name).unwrap();
        (name.to_borrowed(), repo.to_borrowed())
    }

    pub fn new_overlay(
        &mut self,
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'_>,
        options: NewOverlayOptions<'_>,
    ) -> anyhow::Result<(RepoName<'_>, RepoEntry<'_>)> {
        let repo = RepoEntry {
            kind: RepoEntryKind::Overlay {},
        };
        self.validate_no_add_conflicts(dirs, name.to_borrowed(), repo.to_borrowed())?;
        // // TODO: improve diagnostic for repo already existing
        // create_dir(&repo.path(dirs, name.to_borrowed())?) // TODO: revert creating this if something fails
        //     .context("failed to make clone target directory")?;
        let (name, repo) = match options {
            NewOverlayOptions::Clone {
                source,
                no_checkout,
            } => {
                let (name, repo) =
                    self.clone_new(dirs, git, name.into_static(), repo, source.into_static())?;
                match repo
                    .open(git, dirs, name.to_borrowed())
                    .and_then(|mut repo| {
                        repo.reset()
                            .context("failed to execute reset staged changes")?;
                        if !no_checkout {
                            // TODO: check out files
                            repo.restore().context("failed to populate work tree")?;
                        }
                        Ok(())
                    }) {
                    Ok(()) => (),
                    Err(e) => log::warn!("{}", e),
                };
                (name, repo)
            }
            NewOverlayOptions::Init => self.init_new(dirs, git, name.into_static(), repo)?,
        };

        // Tweak bare repo for overlay
        {
            let mut repo = repo.open(git, dirs, name.to_borrowed())?;
            let name: &str = name.as_ref();
            let home = dirs.home_dir_path()?;
            let repo_specific_special_path = |segment| path!(home | segment | name);
            if let Err(e) = repo
                .set_excludes_file(Some(&repo_specific_special_path(".gitignore.d")))
                .context("failed to set Git excludes file")
            {
                log::warn!("{}", e);
            }
            // // TODO: set attributes file
            // if let Err(e) = repo.set_attributes_file(todo!()) {
            //     log::error!("{}", e);
            // }
            // TODO: Looks like we need to set the remote, boo!
        }

        Ok((name, repo))
    }

    pub fn new_standalone(
        &mut self,
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'_>,
        path: Cow<'_, Path>,
        app_info: Option<AppInfo<'_>>,
        options: NewStandaloneOptions<'_>,
    ) -> anyhow::Result<(RepoName<'_>, RepoEntry<'_>)> {
        let repo = |path: &Path| -> anyhow::Result<_> {
            // Git doesn't understand UNC paths, which is what
            // `std::fs::canonicalize` converts paths to on Windows.
            // There's [reasons] for `std` to do this, but in our
            // context, this is undesirable. Try to avoid this using
            // `dunce` if at all possible.
            //
            // [reasons]: https://docs.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation?tabs=cmd
            let path = canonicalize_path(path)?.into();

            // TODO: Check that repo path isn't inside our data dir

            Ok(RepoEntry {
                kind: RepoEntryKind::Standalone { path, app_info },
            })
        };
        // This could be necessary for canonicalizing stuff later, so do it ourselves.
        let create_dir = |path: &Path| -> anyhow::Result<_> {
            let path_parent_is_dir =
                path.parent()
                    .filter(|p| p != &Path::new(""))
                    .map_or(Ok(true), |p| {
                        p.metadata().map(|m| m.is_dir()).with_context(|| {
                            anyhow!("failed to check if parent of {:?} exists", path)
                        })
                    })?;
            if !path_parent_is_dir {
                bail!("path parent is not a directory")
            }
            let res = create_dir(&path);
            if matches!(&res, Err(e) if e.kind() != io::ErrorKind::AlreadyExists) {
                res.context("failed to create target directory")?;
            }
            Ok(())
        };
        match options {
            NewStandaloneOptions::Init => {
                create_dir(&path)?;
                let repo = repo(&path)?;
                Ok(self.init_new(dirs, git, name.into_static(), repo.into_static())?)
            }
            NewStandaloneOptions::Clone { source } => {
                create_dir(&path)?;
                let repo = repo(&path)?;
                Ok(self.clone_new(
                    dirs,
                    git,
                    name.into_static(),
                    repo.into_static(),
                    source.into_static(),
                )?)
            }
            NewStandaloneOptions::Register => {
                let repo = repo(&path)?;
                Self::check_repo_exists(dirs, git, name.to_borrowed(), repo.to_borrowed())?;
                self.validate_no_add_conflicts(dirs, name.to_borrowed(), repo.to_borrowed())?;
                Ok(self.insert(name.into_static(), repo.into_static()))
            }
        }
    }

    fn init_new(
        &mut self,
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'static>,
        repo: RepoEntry<'static>,
    ) -> anyhow::Result<(RepoName<'_>, RepoEntry<'_>)> {
        self.validate_no_add_conflicts(dirs, name.to_borrowed(), repo.to_borrowed())?;

        let path = repo.path(dirs, name.to_borrowed())?;
        git.init(path.as_ref(), repo.kind().into())
            .context("failed to init with Git")?;

        Ok(self.insert(name, repo))
    }

    fn clone_new(
        &mut self,
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'static>,
        repo: RepoEntry<'static>,
        source: RepoSource<'static>,
    ) -> anyhow::Result<(RepoName<'_>, RepoEntry<'_>)> {
        self.validate_no_add_conflicts(dirs, name.to_borrowed(), repo.to_borrowed())?;

        let path = repo.path(dirs, name.to_borrowed())?;
        git.clone(path.as_ref(), source, repo.kind().into())
            .context("failed to clone with Git")?;

        Ok(self.insert(name, repo))
    }

    pub fn validate_no_add_conflicts(
        &mut self,
        dirs: &Directories,
        name: RepoName<'_>,
        repo: RepoEntry<'_>,
    ) -> anyhow::Result<()> {
        let path = repo.path(dirs, name.to_borrowed())?;
        for (other_name, repo) in self.repos.iter() {
            let names_match = &name == other_name;
            let paths_match = {
                let other_repo_path = repo.path(dirs, other_name.to_borrowed())?;
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
                // TODO: These diagnostics should probably live in `runner`. Let's audit diagnostic
                // locations after we get things working.
                if names_match && paths_match {
                    bail!(
                        "repo {:?} is already added; did you accidentally repeat a command?",
                        other_name,
                    );
                } else {
                    bail!(
                        "a repo with the name {:?} already exists as a {}",
                        other_name,
                        repo.short_desc(),
                    );
                }
            }
        }
        Ok(())
    }

    fn check_repo_exists(
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'_>,
        repo: RepoEntry<'_>,
    ) -> anyhow::Result<()> {
        let check = repo.path(dirs, name.to_borrowed()).and_then(|path| {
            git.exists(path.as_ref(), repo.kind().into())
                .context("failed trying to check if Git repo is present at path")
        })?;
        check.context("Git repo check failed")?;
        log::debug!(
            "validated that work tree exists as expected for {}",
            repo.short_desc()
        );
        Ok(())
    }

    pub fn get_by_name_opt(&self, name: RepoName<'_>) -> Option<RepoEntry<'_>> {
        // SAFETY: Safe because we're only using this reference in this call -- no lifetime
        // escaping here.
        {
            let name = &name;
            let name = unsafe { transmute::<_, &RepoName<'static>>(name) };
            self.repos.get(&name)
        }
        .map(|e| e.to_borrowed())
    }

    pub fn get_by_name(&self, name: RepoName<'_>) -> anyhow::Result<RepoEntry<'_>> {
        self.get_by_name_opt(name.to_borrowed())
            .with_context(|| anyhow!("{:?} is not a repo name in the current configuration", name))
    }

    pub fn get_by_path(
        &self,
        dirs: &Directories,
        path: &Path,
    ) -> anyhow::Result<(RepoName<'_>, RepoEntry<'_>)> {
        // TODO: lint/check for canonicalized paths on init
        let path = canonicalize_path(path)?;
        for (name, repo) in self.iter() {
            let repo_path = repo.path(dirs, name.to_borrowed())?;
            if path == repo_path {
                return Ok((name, repo));
            }
        }
        bail!(
            "{:?} is not a path associated with any repo in the current configuration",
            path,
        );
    }

    pub fn iter(&self) -> impl Iterator<Item = (RepoName<'_>, RepoEntry<'_>)> {
        self.repos
            .iter()
            .map(|(name, repo)| (name.to_borrowed(), repo.to_borrowed()))
    }

    pub fn flush(&mut self, dirs: &Directories) -> anyhow::Result<()> {
        let Self {
            repos,
            needs_persist,
        } = self;

        if !*needs_persist {
            return Ok(());
        }

        let standalone_repos = repos
            .iter()
            .filter_map(|(name, entry)| {
                let RepoEntry { kind } = entry;
                match kind {
                    RepoEntryKind::Standalone { app_info, path } => Some((
                        name.to_borrowed(),
                        StandaloneRepoEntry {
                            path: path.to_borrowed(),
                            app_info: app_info.to_borrowed(),
                        },
                    )),
                    RepoEntryKind::Overlay {} => None,
                }
            })
            .collect();

        let standalone_repos_db = StandaloneRepoDb { standalone_repos };

        let toml = toml::to_string(&standalone_repos_db)
            .expect("failed to serialize standalone repos DB as TOML");
        fs::write(dirs.standalone_repo_db_path()?, &toml)
            .context("failed to write standalone repos DB")
    }

    pub fn remove_overlay_bare_repo(
        &mut self,
        dirs: &Directories,
        name: RepoName<'_>,
    ) -> anyhow::Result<()> {
        ensure!(
            self.get_by_name(name.to_borrowed())?.kind() == CliRepoKind::Overlay,
            "repo is not an overlay repo"
        );

        let repo = self.remove(name.to_borrowed()).unwrap();

        remove_dir_all(repo.path(dirs, name)?)
            .context("failed to remove; good luck, you're on your own!")?;

        Ok(())
    }

    pub fn deregister_standalone(
        &mut self,
        name: RepoName<'_>,
    ) -> anyhow::Result<RepoEntry<'static>> {
        ensure!(
            self.get_by_name(name.to_borrowed())?.kind() == CliRepoKind::Standalone,
            "repo is not an standalone repo"
        );
        Ok(self.remove(name).unwrap())
    }

    pub fn try_remove_entire_repo(
        &mut self,
        dirs: &Directories,
        git: &DynGit,
        name: RepoName<'_>,
        // TODO: have an event consumer getting passed in
    ) -> anyhow::Result<RepoEntry<'static>> {
        let repo = self
            .remove(name.to_borrowed())
            .with_context(|| anyhow!("no repo with the name {:?} is configured", name))?;

        // TODO: Seek confirmation. This is dangerous, yo.

        // TODO: Check if there are any uncommitted files or branches, if so,
        // seek confirmation.

        match repo.kind() {
            CliRepoKind::Overlay => {
                // Try to delete all files associated with this repo
                match repo
                    .open(git, dirs, name.to_borrowed())?
                    .list_files()
                    .context("failed to list files")
                {
                    Ok(files) => {
                        for file in files {
                            log::debug!("removing {}", file.display());
                            match remove_file(&file) {
                                Ok(()) => (),
                                Err(e) => {
                                    log::warn!("failed to remove {:?}: {}", file, e)
                                }
                            }
                        }
                    }
                    Err(e) => log::warn!("{}", e),
                }
            }
            CliRepoKind::Standalone => (), // deleting the folder should suffice
        }
        let repo_path = repo.path(dirs, name)?;
        remove_dir_all(&repo_path).with_context(|| {
            anyhow!(
                "failed to delete repo at {:?}; watch out, you're on your own now!",
                repo_path
            )
        })?;
        Ok(repo)
    }

    fn remove(&mut self, name: RepoName<'_>) -> Option<RepoEntry<'static>> {
        let Self {
            repos,
            needs_persist,
        } = self;
        let removed = {
            // SAFETY: Safe because we're only using this reference in this call -- no lifetime
            // escaping here.
            let name = &name;
            let name = unsafe { transmute::<_, &RepoName<'static>>(name) };
            repos.remove(&name)
        };
        *needs_persist = true;
        removed
    }
}

#[derive(Debug)]
pub enum NewStandaloneOptions<'a> {
    Init,
    Clone { source: RepoSource<'a> },
    Register,
}

#[derive(Debug)]
pub enum NewOverlayOptions<'a> {
    Init,
    Clone {
        source: RepoSource<'a>,
        no_checkout: bool,
    },
}

#[derive(Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct StandaloneRepoDb<'a> {
    #[serde(borrow)]
    standalone_repos: BTreeMap<RepoName<'a>, StandaloneRepoEntry<'a>>,
}

#[derive(Debug, Deserialize, Eq, IntoStatic, Ord, PartialEq, PartialOrd, Serialize)]
struct StandaloneRepoEntry<'a> {
    #[serde(borrow)]
    path: Cow<'a, Path>,
    #[serde(borrow)]
    app_info: Option<AppInfo<'a>>,
}

#[derive(Debug, Deserialize, Eq, IntoStatic, Ord, PartialEq, PartialOrd, Serialize, ToBorrowed)]
pub struct AppInfo<'a> {
    qualifier: Cow<'a, str>,
    organization: Cow<'a, str>,
    application: Cow<'a, str>,
}

impl StandaloneRepoDb<'static> {
    fn from_toml_on_disk(dirs: &Directories) -> anyhow::Result<Self> {
        let standalone_repos_db_path = dirs.standalone_repo_db_path()?;
        log::trace!(
            "reading standalone repos DB at {}",
            standalone_repos_db_path.display()
        );
        let db_toml = {
            let mut buf = String::new();
            let mut reader = BufReader::new(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&standalone_repos_db_path)
                    .with_context(|| {
                        anyhow!(
                            "failed to open standalone repos DB at {}",
                            standalone_repos_db_path.display(),
                        )
                    })?,
            );
            reader.read_to_string(&mut buf).with_context(|| {
                anyhow!(
                    "failed to read standalone repos DB at {}",
                    standalone_repos_db_path.display()
                )
            })?;
            buf
        };
        let parsed = StandaloneRepoDb::from_toml(&db_toml).with_context(|| {
            anyhow!(
                "failed to deserialize TOML from standalone repo DB at {}",
                standalone_repos_db_path.display(),
            )
        })?;
        Ok(parsed.into_static())
    }
}

impl<'a> StandaloneRepoDb<'a> {
    fn into_static(self) -> StandaloneRepoDb<'static> {
        let Self { standalone_repos } = self;

        StandaloneRepoDb {
            standalone_repos: standalone_repos
                .into_iter()
                .map(|(name, entry)| (name.into_static(), entry.into_static()))
                .collect(),
        }
    }

    fn into_runner_repos(self) -> impl Iterator<Item = (RepoName<'a>, RepoEntry<'a>)> {
        let Self { standalone_repos } = self;

        standalone_repos
            .into_iter()
            .map(|(name, StandaloneRepoEntry { app_info, path })| {
                (
                    name,
                    RepoEntry {
                        kind: RepoEntryKind::Standalone { path, app_info },
                    },
                )
            })
    }
}

impl<'a> StandaloneRepoDb<'a> {
    fn from_toml(db_toml: &'a str) -> anyhow::Result<Self> {
        if db_toml.trim().is_empty() {
            Ok(StandaloneRepoDb::default())
        } else {
            // TODO: Validate duplicate entry handling.
            Ok(toml::from_str(db_toml)?)
        }
    }
}
