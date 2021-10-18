# Capisco

A dotfiles manager that understands you.

Capisco is a Git repository manager oriented towards cross-platform dotfiles.
It draws heavy inspiration from [`vcsh`](https://github.com/RichiH/vcsh), which
focuses on managing local repositories, and otherwise leaves the history of
your dotfiles repository(ies) to Git. Here's an example of Capisco (the `cpsc`
binary) in action:

```sh
# Start from scratch by adding some existing repos of yours. Since you probably
# have a normal Git repo or two lying around (which would be "standalone" repos
# in Capisco terms), let's use those!
$ cpsc standalone register ./my-first-repo
$ cpsc standalone register ./my-second-repo

# 👀 We can see that Capisco registered these repos with a `list` subcommand:
$ cpsc list

# Run `git push` on each repo we've configured in Capisco.
$ cpsc for-each -- git push
```

Ready to dive in deeper? You should try:

* The [User Guide](./docs/user-guide/src/introduction.md) for guide-level documentation.
* See the output `cpsc help` for reference-level documentation, per command and subcommand.

## Installation

For now, you may build Capisco via [Cargo](https://doc.rust-lang.org/cargo/):

```sh
$ cargo install capisco
```

Binary distributions in GitHub and your favorite package manager(s) are coming
soon (see also [the roadmap](#roadmap))!

## Roadmap

See Capisco's [milestones in
GitHub](https://github.com/capisco-dotfiles/capisco/milestones) for Capisco's
current roadmap.

## Contributing

See the [contributor book](./docs/contributor-book/src/welcome.md)!
