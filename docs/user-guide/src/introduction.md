# Introduction

This book offers guide-level documentation for
[Bellboy](https://github.com/bellboy-dotfiles/bellboy), a cross-platform
dotfiles manager that understands you.

🚧 This documentation is still **under construction**. 🚧 Stay tuned for
[project releases](https://github.com/bellboy-dotfiles/bellboy/releases), where
we'll be adding more content here soon!

---

There are three key workflows that Bellboy offers:

1. `standalone` repos. These are typical Git repos, with some extra
	declarative configuration so that you don't have to think about where to
	`git clone` your dotfiles between platforms.
2. `overlay` repos, which are Git repos that:
	1. are rooted root at your home directory, and
	2. potentially overlapping work trees.

	This lets you keep configuration separate, even between different
	applications that use the home directory for their files. For instance, all
	of the following applications have (at least default) config paths at the
	home directory:

	1. Git has `~/.gitconfig`.
	2. Vim has `~/.vimrc`.
	3. Bash and ZSH have `.bashrc` and `~/.zshrc`
	4. `tmux` has a `~/.tmux.conf`.

	...and the list goes on. Wouldn't it be nice to have the flexibility of
	keeping these in separate repos? Bellboy can do that for you!
3. A nice command line interface for managing your repos via their assigned names.
	This lets you do powerful things like check the status of _each_ of your
	repos, or push their changes up to Git remotes, with a single command:

	```sh
	# Example: push up all changes for each repo
	$ bb for-each -- git push
	```

All of these culminate in an experience that we think is too good to not share!
