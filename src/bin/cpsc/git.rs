use std::{
    borrow::Cow,
    convert::Infallible,
    ffi::OsStr,
    fmt::Debug,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};
use thiserror::Error as ThisError;

pub use cli::GitCli;

pub trait GitTrait
where
    Self: Debug,
{
    type Repo: GitRepoTrait;

    fn exists(
        &self,
        path: &Path,
        repo_kind: GitRepoKind,
    ) -> Result<Result<(), GitExistCheckFailure>, GitExistError>;

    fn clone(
        &self,
        path: &Path,
        source: RepoSource<'_>,
        repo_kind: GitRepoKind,
    ) -> Result<(), GitCloneError>;

    fn open_repo(&self, options: OpenRepoOptions<'_>) -> Result<Self::Repo, OpenRepoError>;
}

pub trait GitRepoTrait {
    type ListFilesIter: Iterator<Item = PathBuf>;

    fn run_cmd<T>(&self, cmd: Command, f: impl FnOnce(Command) -> T) -> T;
    fn set_excludes_file(&mut self, path: Option<&Path>) -> Result<(), GitSetExcludeFileError>;
    fn list_files(&self) -> Result<Self::ListFilesIter, GitListFilesError>;
}

pub enum OpenRepoOptions<'a> {
    Bare {
        repo_path: &'a Path,
        work_tree_path: &'a Path,
    },
    Normal {
        work_tree_path: &'a Path,
    },
}

#[derive(Debug)]
pub enum DynGit {
    Cli(GitCli),
}

pub enum DynGitRepo {
    Cli(<GitCli as GitTrait>::Repo),
}

impl GitTrait for DynGit {
    type Repo = DynGitRepo;

    fn exists(
        &self,
        path: &Path,
        repo_kind: GitRepoKind,
    ) -> Result<Result<(), GitExistCheckFailure>, GitExistError> {
        match self {
            Self::Cli(cli) => cli.exists(path, repo_kind),
        }
    }

    fn clone(
        &self,
        path: &Path,
        source: RepoSource<'_>,
        repo_kind: GitRepoKind,
    ) -> Result<(), GitCloneError> {
        match self {
            Self::Cli(cli) => cli.clone(path, source, repo_kind),
        }
    }

    fn open_repo(&self, options: OpenRepoOptions<'_>) -> Result<Self::Repo, OpenRepoError> {
        match self {
            Self::Cli(cli) => Ok(DynGitRepo::Cli(cli.open_repo(options)?)),
        }
    }
}

impl GitRepoTrait for DynGitRepo {
    type ListFilesIter = Box<dyn Iterator<Item = PathBuf>>;

    fn run_cmd<T>(&self, cmd: Command, f: impl FnOnce(Command) -> T) -> T {
        match self {
            Self::Cli(cli) => cli.run_cmd(cmd, f),
        }
    }

    fn set_excludes_file(&mut self, path: Option<&Path>) -> Result<(), GitSetExcludeFileError> {
        match self {
            Self::Cli(cli) => cli.set_excludes_file(path),
        }
    }

    fn list_files(&self) -> Result<Self::ListFilesIter, GitListFilesError> {
        match self {
            Self::Cli(cli) => cli.list_files(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RepoSource<'a>(Cow<'a, str>);

impl AsRef<OsStr> for RepoSource<'_> {
    fn as_ref(&self) -> &OsStr {
        let Self(inner) = self;
        OsStr::new(inner.as_ref())
    }
}

impl FromStr for RepoSource<'static> {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Cow::Owned(s.to_string())))
    }
}

#[derive(Debug, ThisError)]
#[error("failed to check that a Git repo exists at {}: {op}", path.display())]
pub struct GitExistError {
    op: Cow<'static, str>,
    path: PathBuf,
    source: Option<anyhow::Error>,
}

#[derive(Debug, ThisError)]
#[error("expected {expected:?}, got {actual:?}")]
pub struct GitExistCheckFailure {
    expected: GitRepoKind,
    actual: Option<GitRepoKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitRepoKind {
    Normal,
    Bare,
}

#[derive(Debug, ThisError)]
#[error("failed to clone Git repo from {source:?} into {}: {op}", path.display())]
pub struct GitCloneError {
    op: Cow<'static, str>,
    path: PathBuf,
    source: Option<anyhow::Error>,
}

const EXCLUDES_FILE_CONFIG_PATH: &str = "core.excludesFile";

#[derive(Debug, ThisError)]
#[error("failed to set `{}` config", EXCLUDES_FILE_CONFIG_PATH)]
pub struct GitSetExcludeFileError(#[from] anyhow::Error);

#[derive(Debug, ThisError)]
#[error("failed to open repo at {}", path.display())]
pub struct OpenRepoError {
    path: PathBuf,
    source: anyhow::Error,
}

#[derive(Debug, ThisError)]
#[error("failed to list files")]
pub struct GitListFilesError {
    source: anyhow::Error,
}

fn prep_cmd<'a>(cmd: &mut Command, git_work_tree_path: &Path, git_dir_path: &Path) {
    cmd.envs([
        ("GIT_WORK_TREE", (&*git_work_tree_path).as_os_str()),
        ("GIT_DIR", (&*git_dir_path).as_os_str()),
    ]);
}

mod cli {
    use super::{
        prep_cmd, GitCloneError, GitExistCheckFailure, GitExistError, GitListFilesError,
        GitRepoKind, GitRepoTrait, GitSetExcludeFileError, GitTrait, OpenRepoError,
        OpenRepoOptions, RepoSource, EXCLUDES_FILE_CONFIG_PATH,
    };
    use anyhow::{anyhow, ensure, Context};
    use std::{
        borrow::Cow,
        ffi::OsStr,
        io::{BufRead, Cursor},
        path::{Path, PathBuf},
        process::{Command, ExitStatus, Output, Stdio},
    };

    // TODO: use `GIT_REFLOG_ACTION` for logging niceness

    #[derive(Debug)]
    pub struct GitCli;

    #[derive(Debug)]
    pub struct GitCliRepo {
        work_tree_path: PathBuf,
        repo_path: PathBuf,
    }

    impl GitCli {
        fn cmd_failure_err(status: ExitStatus) -> Option<Cow<'static, str>> {
            match status.code() {
                Some(0) => None,
                Some(code) => {
                    Some(format!("exited with exit status {}, see output above", code).into())
                }
                None => Some("command was terminated by a signal".into()),
            }
        }
    }

    impl GitTrait for GitCli {
        type Repo = GitCliRepo;

        fn exists(
            &self,
            path: &Path,
            expected_repo_kind: GitRepoKind,
        ) -> Result<Result<(), GitExistCheckFailure>, GitExistError> {
            let err = |op, source| GitExistError {
                op,
                path: path.to_owned(),
                source,
            };

            let Output {
                stdout,
                stderr,
                status,
            } = Command::new("git")
                .args::<_, &OsStr>([
                    "-C".as_ref(),
                    path.as_ref(),
                    "rev-parse".as_ref(),
                    "--is-bare-repository".as_ref(),
                ])
                .output()
                .map_err(|e| {
                    err(
                        "unable to spawn command".into(),
                        Some(anyhow::Error::new(e)),
                    )
                })?;

            let parse_std = |channel_name, channel| {
                String::from_utf8(channel).map_err(|e| {
                    err(
                        format!("failed to parse `rev-parse`'s `{}` as UTF-8", channel_name,)
                            .into(),
                        Some(anyhow::Error::new(e)),
                    )
                })
            };

            let stderr = parse_std("stderr", stderr)?;

            let actual =
                if status.code() == Some(128) && stderr.find("not a git repository").is_some() {
                    // TODO: how to make this `None` check more stable?
                    None
                } else if let Some(err_msg) = Self::cmd_failure_err(status) {
                    eprintln!("{}", stderr);
                    return Err(err(err_msg, None));
                } else {
                    let found = parse_std("stdout", stdout)?
                        .trim()
                        .parse::<bool>()
                        .map(|b| {
                            if b {
                                GitRepoKind::Bare
                            } else {
                                GitRepoKind::Normal
                            }
                        })
                        .map_err(|e| {
                            err(
                                "failed to parse `rev-parse` response as a boolean literal".into(),
                                Some(anyhow::Error::new(e)),
                            )
                        })?;
                    Some(found)
                };

            Ok(if Some(expected_repo_kind) == actual {
                Ok(())
            } else {
                Err(GitExistCheckFailure {
                    expected: expected_repo_kind,
                    actual,
                })
            })
        }

        fn clone(
            &self,
            path: &Path,
            source: RepoSource<'_>,
            repo_kind: GitRepoKind,
        ) -> Result<(), GitCloneError> {
            let err = |op, source| GitCloneError {
                op,
                path: path.to_owned(),
                source,
            };

            let mut git_cmd = Command::new("git");
            git_cmd.args::<_, &OsStr>(["clone".as_ref(), source.as_ref(), path.as_ref()]);
            match repo_kind {
                GitRepoKind::Normal => (),
                GitRepoKind::Bare => {
                    git_cmd.arg("--bare");
                }
            }

            let status = git_cmd
                .status()
                .map_err(|e| err("spawn command".into(), Some(anyhow::Error::new(e))))?;

            if let Some(err_msg) = Self::cmd_failure_err(status) {
                Err(err(err_msg, None))
            } else {
                Ok(())
            }

            // TODO: Track HEAD branch against `origin`?
            // TODO: `git reset`?
        }

        fn open_repo(&self, options: OpenRepoOptions<'_>) -> Result<Self::Repo, OpenRepoError> {
            let exists = |path, kind| {
                self.exists(path, kind)
                    .map_err(|e| anyhow::Error::new(e))
                    .and_then(|res| Ok(res?))
                    .map_err(|source| OpenRepoError {
                        path: path.to_owned(),
                        source: source.into(),
                    })
            };
            match options {
                OpenRepoOptions::Bare {
                    repo_path,
                    work_tree_path,
                } => exists(repo_path, GitRepoKind::Bare).map(|()| GitCliRepo {
                    repo_path: repo_path.to_owned(),
                    work_tree_path: work_tree_path.to_owned(),
                }),
                OpenRepoOptions::Normal { work_tree_path } => {
                    exists(work_tree_path, GitRepoKind::Normal).map(|()| GitCliRepo {
                        repo_path: work_tree_path.join(".git"),
                        work_tree_path: work_tree_path.to_owned(),
                    })
                }
            }
        }
    }

    impl GitCliRepo {
        fn git_cmd() -> Command {
            Command::new("git")
        }
    }

    impl GitRepoTrait for GitCliRepo {
        type ListFilesIter = Box<dyn Iterator<Item = PathBuf>>;

        fn run_cmd<T>(&self, mut cmd: Command, f: impl FnOnce(Command) -> T) -> T {
            let Self {
                work_tree_path,
                repo_path,
            } = &self;
            prep_cmd(&mut cmd, work_tree_path, repo_path);
            f(cmd)
        }

        fn set_excludes_file(&mut self, path: Option<&Path>) -> Result<(), GitSetExcludeFileError> {
            let mut cmd = Self::git_cmd();
            cmd.arg("config");
            if let Some(path) = path {
                cmd.args(["set", "core.excludesFile", EXCLUDES_FILE_CONFIG_PATH]);
                cmd.arg(path);
            } else {
                cmd.args(["--unset", EXCLUDES_FILE_CONFIG_PATH]);
            }

            let exit_status = self
                .run_cmd(cmd, |mut cmd| cmd.status())
                .context("failed to spawn command")?;
            if !exit_status.success() {
                return Err(anyhow!("command did not exit successfully").into());
            }
            Ok(())
        }

        fn list_files(&self) -> Result<Self::ListFilesIter, GitListFilesError> {
            let mut cmd = Command::new("git");
            cmd.arg("ls-files").stderr(Stdio::inherit());
            (|| {
                let Output {
                    status,
                    stdout,
                    stderr: _,
                } = self
                    .run_cmd(cmd, |mut cmd| cmd.output())
                    .context("failed to spawn file listing command")
                    .map_err(|source| GitListFilesError { source })?;
                ensure!(status.success(), "command did not exit with 0");
                let files = BufRead::lines(Cursor::new(stdout))
                    .map(|l| {
                        l.context("failed to read line from output")
                            .map(|l| Path::new(&l).to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(files.into_iter())
            })()
            .map(|i| -> Box<dyn Iterator<Item = PathBuf>> { Box::new(i) })
            .map_err(|source| GitListFilesError { source })
        }
    }
}
