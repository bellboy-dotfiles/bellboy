use self::normalization::Normalization;
use crate::{
    cli::CliRepoKind,
    runner::{
        repo_db::{conflict::normalization::NormalizedEqOutcome, RepoDb, RepoEntry, RepoName},
        Directories,
    },
};
use anyhow::anyhow;
use lifetime::{IntoStatic, ToBorrowed};
use same_file::is_same_file;
use std::{
    borrow::Cow,
    convert::Infallible,
    fmt::{self, Formatter},
    fs, io,
    path::Path,
};
use unicase::UniCase;

pub mod normalization;

pub(crate) struct RepoConflictSearcher<'a> {
    search_name: RepoName<'a>,
    search_path: Cow<'a, Path>,
    dirs: &'a Directories,
    iter: Box<dyn Iterator<Item = (&'a RepoName<'a>, &'a RepoEntry<'a>)> + 'a>,
}

impl<'a> RepoConflictSearcher<'a> {
    pub(in crate::runner) fn new(
        name: RepoName<'a>,
        entry: RepoEntry<'a>,
        dirs: &'a Directories,
        repo_db: &'a RepoDb,
    ) -> anyhow::Result<Self> {
        // TODO: Check for a `standalone` repo path within our local data dir -- don't allow this.
        let search_path = entry.path(dirs, name.to_borrowed())?.into_static();
        Ok(RepoConflictSearcher {
            search_name: name,
            search_path,
            dirs,
            iter: Box::new(repo_db.repos.iter()),
        })
    }

    pub fn next_conflict(&mut self) -> Option<anyhow::Result<RepoConflictCheck<'_>>> {
        let Self {
            dirs,
            iter,
            search_name,
            search_path,
        } = self;

        let (other_name, repo) = iter.next()?;

        (move || {
            let name_eq = {
                let outcome = NormalizedRepoNameEq::normalized_eq(search_name, other_name).unwrap();
                RepoFieldEq {
                    found: other_name.to_borrowed(),
                    outcome,
                }
            };

            let entry_match = {
                let other_repo_path = repo.path(dirs, other_name.to_borrowed())?;
                // TODO: Resolve Git repo root (incl. w/ worktrees).
                // TODO: Do we need `is_same_file` if we canonicalize?
                // TODO (DONE?): add case that checks for a non-existent repo path -- we should
                // warn the user that their repo is gone!
                //
                // For other errors, bail out. We can't make any guarantees about maintaining
                // integrity of our configuration if we encounter those errors.
                let outcome =
                    if other_repo_path.exists() || matches!(repo.kind(), CliRepoKind::Overlay) {
                        NormalizedRepoPathEq::normalized_eq(search_path, &other_repo_path)?
                    } else {
                        log::warn!("Git work tree directory of existing {}", repo.short_desc());
                        NormalizedEqOutcome::NotAMatch
                    };

                RepoFieldEq {
                    found: other_repo_path,
                    outcome,
                }
            };
            if name_eq.outcome.matched() || entry_match.outcome.matched() {
                return Ok(Some(RepoConflictCheck {
                    found_name: other_name.to_borrowed(),
                    name_eq,
                    entry_match,
                }));
            }

            Ok(None)
        })()
        .transpose()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RepoConflictCheck<'a> {
    pub found_name: RepoName<'a>,
    pub name_eq: RepoFieldEq<RepoName<'a>, NormalizedRepoNameEq>,
    pub entry_match: RepoFieldEq<Cow<'a, Path>, NormalizedRepoPathEq>,
}

#[derive(Clone, Debug)]
pub struct RepoFieldEq<T, R>
where
    R: Normalization<T>,
{
    pub found: T,
    pub outcome: NormalizedEqOutcome<R>,
}

#[derive(Clone, Copy, Debug)]
pub enum NormalizedRepoNameEq {
    CaseInsensitiveMatch,
}

impl<'a> Normalization<RepoName<'a>> for NormalizedRepoNameEq {
    type Error = Infallible;

    fn normalized_eq(
        t1: &RepoName<'_>,
        t2: &RepoName<'_>,
    ) -> Result<NormalizedEqOutcome<Self>, Self::Error> {
        Ok(if t1 == t2 {
            NormalizedEqOutcome::ExactMatch
        } else if UniCase::new(&**t1) == UniCase::new(&**t2) {
            NormalizedEqOutcome::MatchAfterNormalization {
                reason: NormalizedRepoNameEq::CaseInsensitiveMatch,
            }
        } else {
            NormalizedEqOutcome::NotAMatch
        })
    }

    fn describe(&self, t: &RepoName<'_>, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CaseInsensitiveMatch => {
                write!(f, "matches {t:?} case-insensitively")
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum NormalizedRepoPathEq {
    CanonicalizedPathsEqual,
}

impl<'a> Normalization<Cow<'a, Path>> for NormalizedRepoPathEq {
    type Error = anyhow::Error;

    fn normalized_eq(
        t1: &Cow<'a, Path>,
        t2: &Cow<'a, Path>,
    ) -> Result<NormalizedEqOutcome<Self>, Self::Error> {
        let exists = |path| {
            fs::metadata(path).map(|_metadata| true).or_else(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(false)
                } else {
                    Err(anyhow!("failed to check if path {path:?} exists: {e}"))
                }
            })
        };
        let t1_exists = exists(&t1)?;
        let t2_exists = exists(&t2)?;
        let is_same_file = match (t1_exists, t2_exists) {
            (false, false) => t1 == t2,
            (false, true) | (true, false) => false,
            (true, true) => is_same_file(t1, t2).map_err(|e| {
                anyhow!(
                    "failed to compare paths for equality: {:?}, {:?}: {}",
                    t1,
                    t2,
                    e,
                )
            })?,
        };

        Ok(if is_same_file {
            if t1 == t2 {
                NormalizedEqOutcome::ExactMatch
            } else {
                NormalizedEqOutcome::MatchAfterNormalization {
                    reason: NormalizedRepoPathEq::CanonicalizedPathsEqual,
                }
            }
        } else {
            NormalizedEqOutcome::NotAMatch
        })
    }

    fn describe(&self, t: &Cow<'_, Path>, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalizedPathsEqual => {
                write!(f, "is the same path as {t:?} when canonicalized")
            }
        }
    }
}

pub trait RepoConflictHandler {
    fn on_conflict_path(
        &mut self,
        matched: RepoName<'_>,
        partial_reason: Option<(Cow<'_, Path>, NormalizedRepoPathEq)>,
    );

    fn on_conflict_name(
        &mut self,
        matched: RepoName<'_>,
        partial_reason: Option<NormalizedRepoNameEq>,
    );

    fn on_iteration_err(&mut self, err: anyhow::Error);
}
