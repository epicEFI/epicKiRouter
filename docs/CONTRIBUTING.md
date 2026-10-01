# Introduction

First off, thank you for considering contributing to Freerouting. It's people like you that make Freerouting such a great tool.

Following these guidelines helps to communicate that you respect the time of the developers managing and developing this open source project. In return, they should reciprocate that respect in addressing your issue, assessing changes, and helping you finalize your pull requests.

Freerouting is an open source project and we love to receive contributions from our community — you! There are many ways to contribute, from writing tutorials or blog posts, improving the documentation, submitting bug reports and feature requests or writing code which can be incorporated into Freerouting itself.

**Note on UI translations:** the Java-era LLM translation pipeline (`scripts/i18n/`) was retired with the M10-T4 Java sunset; the historical translation workflow is preserved in git history.

# Ground Rules

Responsibilities
* Ensure cross-platform compatibility for every change that's accepted. Windows, Mac, Debian & Ubuntu Linux.
* Create issues for any major changes and enhancements that you wish to make. Discuss things transparently and get community feedback.
* Be welcoming to newcomers and encourage diverse new contributors from all backgrounds.

# Your First Contribution

Unsure where to begin contributing to Freerouting? You can start by looking through these curated issues:
- [Good first issues](https://github.com/freerouting/freerouting/labels/good%20first%20issue) - issues which should only require a few lines of code, and a test or two.
- [Help wanted issues](https://github.com/freerouting/freerouting/labels/help%20wanted) - issues where community help is actively requested.

For a comprehensive guide to all issue and PR labels, see [`docs/labels.md`](labels.md).

### Bonus points: Add a link to a resource for people who have never contributed to open source before.

Working on your first Pull Request? You can learn how from this *free* series, [How to Contribute to an Open Source Project on GitHub](https://egghead.io/series/how-to-contribute-to-an-open-source-project-on-github) and here are a couple of friendly tutorials you can check out: http://makeapullrequest.com/ and http://www.firsttimersonly.com/.

At this point, you're ready to make your changes! Feel free to ask for help; everyone is a beginner at first :smile_cat:

If a maintainer asks you to "rebase" your PR, they're saying that a lot of code has changed, and that you need to update your branch so it's easier to merge.

# Getting started

For something that is bigger than a one or two line fix:

1. Create your own fork of the code
2. Do the changes in your fork
3. If you like the change and think the project could use it:
    * Be sure you have followed the code style for the project.
    * Note the Freerouting Code of Conduct.
    * Send a pull request.

## Code quality and formatting

EpicRouter (the Rust rewrite of Freerouting) uses `cargo fmt`, `cargo clippy`, the Rust
test census, pre-commit hooks, and GitHub Actions. Formatting and validation are part of
the contribution contract: a change that fails these checks is not ready to commit.

Install the local checks once:

```bash
python -m pip install pre-commit
pre-commit install
```

Run the same checks locally before committing (the Rust tree is `rust/`; run cargo from
there):

```bash
pre-commit run --all-files
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The generic hygiene hooks automatically repair trailing whitespace and LF line endings
only in files selected for the current commit; the final-newline repair is MANUAL-STAGE:
the hook fixes the working tree, and the repaired content is staged by you (never
auto-staged — a failed repair of a file you did not intend to touch must not slip into
your commit). Review and stage those changes, then rerun the hook when it reports that
files were modified.

The repository uses LF for source, build, metadata, and documentation files on every
platform through `.gitattributes`. Do not change `core.autocrlf` back and forth to fix a
single working tree. If the repository needs one-time normalization, do it in a dedicated
commit with no unrelated changes.

GitHub Actions runs the quality gates (`rust-check.yml` on `rust/**` changes; the
pre-commit workflow on `main` and `epic/main`). A contributor should resolve local fmt, clippy, test, or
line-ending failures before opening or updating a pull request.

As a rule of thumb, changes are obvious fixes if they do not introduce any new functionality or creative thinking. As long as the change does not affect functionality, some likely examples include the following:
* Spelling / grammar fixes
* Typo correction, white space and formatting changes
* Comment clean up
* Bug fixes that change default return values or error codes stored in constants
* Adding logging messages or debugging output
* Changes to ‘metadata’ files like .gitignore, build scripts, etc.
* Moving source files from one directory or package to another

# How to report a bug

 When filing an issue, make sure to answer these five questions:

 1. What version of Freerouting are you using?
 2. What operating system and processor architecture are you using?
 3. What did you do?
 4. What did you expect to see?
 5. What did you see instead?

If you find yourself wishing for a feature that doesn't exist in Freerouting, you are probably not alone. There are bound to be others out there with similar needs. Many of the features that Freerouting has today have been added because our users saw the need. Open an issue on our issues list on GitHub which describes the feature you would like to see, why you need it, and how it should work.
