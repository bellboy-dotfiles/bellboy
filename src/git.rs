use std::{
    borrow::Cow,
    convert::Infallible,
    ffi::OsStr,
    fmt::{self, Debug, Display, Formatter},
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error as ThisError;

pub use cli::GitCli;

pub trait Git
where
    Self: Debug,
{
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

#[derive(Debug)]
pub struct GitExistCheckFailure {
    expected: GitRepoKind,
    actual: Option<GitRepoKind>,
}

impl Display for GitExistCheckFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let Self { expected, actual } = self;
        write!(f, "expected {:?}, got {:?}", expected, actual)
    }
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

mod cli {
    use super::{Git, GitCloneError, GitExistCheckFailure, GitExistError, GitRepoKind, RepoSource};
    use std::{
        borrow::Cow,
        ffi::OsStr,
        path::Path,
        process::{Command, ExitStatus, Output},
    };

    // TODO: use `GIT_REFLOG_ACTION` for logging niceness

    #[derive(Debug)]
    pub struct GitCli;

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

    impl Git for GitCli {
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

            // TODO: `git reset`?
            // TODO: Track HEAD branch against `origin`?
        }
    }
}
