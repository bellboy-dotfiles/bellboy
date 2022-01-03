# Capisco 💡☝️

A cross-platform dotfiles manager that understands you.

Capisco is a Git repository manager oriented towards cross-platform dotfiles.
It draws heavy inspiration from [`vcsh`], which focuses on managing local
repositories, and otherwise leaves the history of your dotfiles repository(ies)
to Git. Here's an example of Capisco (the `cpsc` binary) in action:

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

See the [Contributor Book]!

[Contributor Book]: ./docs/contributor-book/src/welcome.md

## Credits

Much of the inspiration for this project's design of `overlay` repos is from
[`vcsh`], which you are encouraged to check out if you always use `bash` as
your shell, or always have it available in your command-line.

[`vcsh`]: https://github.com/RichiH/vcsh

## License

This project uses GPL 3.0. If you're curious about rationale, check out the
["License: why GPL 3.0?"
page](./docs/contributor-book/src/license-why-gpl-3.0.md) from the [Contributor
Book].
