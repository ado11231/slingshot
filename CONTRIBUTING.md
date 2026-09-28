# Contributing

* How to set up Slingshot, make a change, and get it merged.
* Slingshot is built for people who have never seen it. Every change should make it easier to install, understand, and trust.

## Contents

1. [Before You Start](#before-you-start)
2. [Setting Up](#setting-up)
3. [Building And Testing](#building-and-testing)
4. [Testing On Two Machines](#testing-on-two-machines)
5. [Making A Change](#making-a-change)
6. [Using AI Tools](#using-ai-tools)
7. [Code Standards](#code-standards)
8. [Output Standards](#output-standards)
9. [Writing Documentation](#writing-documentation)
10. [Commits And Pull Requests](#commits-and-pull-requests)
11. [Out Of Scope](#out-of-scope)
12. [Reporting A Problem](#reporting-a-problem)

## Before You Start

1. Read these documents:

   | Document | Explains |
   | --- | --- |
   | [ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the parts fit together, and what every file does. |
   | [USAGE.md](docs/USAGE.md) | Every command, as a user sees it. |
   | [ROADMAP.md](docs/ROADMAP.md) | What works, what has been tested, and what comes next. |

2. For anything larger than a small fix, open an issue first and describe the change, so the approach is agreed before you write code.

## Setting Up

* You need:

| Tool | Machine | Purpose |
| --- | --- | --- |
| Rust, latest stable | Both | Builds Slingshot. |
| `rsync` | Both | Copies project files. |
| An ssh server | Agent | Carries all work and requests. |
| `tmux` | Agent | Keeps sessions running. |
| Xcode command line tools | Mac Client | Builds the menu bar app. |

1. Download and build the code:

   ```sh
   git clone https://github.com/ado11231/slingshot.git
   cd slingshot
   cargo build
   ```

2. Install your build as the `slingshot` command:

   ```sh
   cargo install --path crates/slingshot-cli
   ```

3. On a Mac, build and open the menu bar app:

   ```sh
   slingshot menubar
   ```

   * Run it again after changing the app. It rebuilds whenever the app's source changed.

## Building And Testing

* Run all three checks before you push. All three must pass with no warnings.

  ```sh
  cargo fmt --check
  cargo clippy --workspace --all-targets
  cargo test --workspace
  ```

* If you changed the menu bar app, also run:

  ```sh
  swift build -c release --package-path mac/menubar
  ```

* CI runs the same checks, and the menu bar build, on Linux and macOS for every pull request.

* **Where tests live:**

1. Unit tests sit at the bottom of the file they test, in a `tests` module.
2. Tests that run the finished program live in `crates/slingshot-cli/tests/`.

* **What needs a test:**

1. Every rule or decision, such as which files are source, how a sync decides what to copy, when a notification is sent, or how an error is explained.
2. Code that only starts another program, such as `ssh`, is tested on real machines instead.

## Testing On Two Machines

* Automated tests cannot prove Slingshot works. Many problems only appear with a real network and a real second machine.
* If your change affects running commands, syncing, sessions, or connections, test it on two machines:

1. Start the Agent with `slingshot start`.
2. Link the Client with `slingshot link <code>`.
3. Use the feature you changed, such as `slingshot run cargo build`, `slingshot sync`, or `slingshot attach`.
4. Test failures: stop `slingshot start`, drop the network, or restart the Agent.

* In your pull request, list exactly what you ran and what happened. Never describe something as working unless you ran it.

## Making A Change

1. **Find the right place.** Use the [file reference](docs/ARCHITECTURE.md#file-reference).
   * Rules both machines need go in `slingshot-core`.
   * Code only the Agent runs goes in `slingshot-agent`.
   * Commands and Client code go in `slingshot-cli`.
2. **Keep it focused.** Make the smallest change that solves the problem. Save unrelated improvements for another pull request.
3. **Match what is there.** Follow the surrounding code's naming, structure, and comment style.
4. **Test it.** Add or update tests for every rule you change.
5. **Run it.** Use the changed command at least once.
6. **Update the documentation:**

   | If You | Update |
   | --- | --- |
   | Add or move a file | The [file reference](docs/ARCHITECTURE.md#file-reference) |
   | Change a command or option | [USAGE.md](docs/USAGE.md) |
   | Add or change an error message | [TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) |
   | Change what works, or test something new | [ROADMAP.md](docs/ROADMAP.md) |

## Using AI Tools

* AI coding tools are welcome. The same standards apply to every line, whoever wrote it.
* **You are the author.** Opening a pull request makes you responsible for every line in it.
* **Understand what you submit.** You must be able to explain what each change does and why. If you cannot, the pull request will be closed.
* **Verify it yourself.** Run the checks and the feature, and read the output. AI tools often call functions that do not exist, miss edge cases, or change more than asked.
* **Keep it small.** Large changes are cheap to generate and expensive to review.
* **Remove the noise.** Delete unused code, obvious comments, and anything the change does not need.
* AI agents working in this repository directly must also follow [CLAUDE.md](CLAUDE.md).

## Code Standards

### Structure

* Dependencies flow one way: `slingshot-cli` uses `slingshot-agent` and `slingshot-core`, and `slingshot-agent` uses `slingshot-core`.
* Wrap existing tools. Slingshot uses `ssh`, `rsync`, `tmux`, `docker`, and `iroh`, and never writes its own versions.
* Describe machines as Client and Agent, never as Mac and Linux. Only the menu bar app may be macOS specific.
* Never write fixed paths such as `/Users/...` or `/home/...`. Use the `directories` crate.
* Never assume a package manager. Detect what is missing and print the install command. Never install without asking: show exactly what will run, then ask once.

### Rust

* Errors use `anyhow`. Every visible error says what went wrong and how to fix it.
* A failing user command is not a Slingshot error. Report its exit code faithfully.
* Use `tokio` for async code, `tracing` for logs, and `clap` for command line parsing.
* Never use `unwrap` outside tests. Use `?`, or `expect` with the reason failure is impossible.
* Quote every argument sent to the Agent on its own. Never join them into one shell command.
* Never stop a process by its ID alone. Check its start time too.
* Changing a control message raises `control::VERSION`. Changing the menu bar line format raises `watch::event::VERSION`.

### Comments

* Add a short comment above an item when its purpose is not obvious. Explain why, not what.
* Keep comments out of function bodies.
* No commented out code, and no `TODO` notes without an issue number.

### Dependencies

* Explain every new crate in the pull request, and use its latest version from crates.io.
* Prefer wrapping an existing tool over adding a large dependency.

### The Menu Bar App

* Swift only displays information and posts notifications. All thresholds and rules stay in Rust, in `crates/slingshot-cli/src/watch/`.
* `Sources/Model.swift` must match `watch/event.rs`. Change them together.

## Output Standards

* Use the helpers in `slingshot-core/src/presentation.rs` and `slingshot-core/src/step.rs`. Never use raw color codes.
* Slow work shows a spinner, then a line such as `✓ Synced 3 files  0.4s`.
* Status symbols: `✓` success, `!` warning, `✗` failure, `▶` remote work.
* Every remote command states where it runs, such as `▶ Running on archbox via tailnet`.
* Color only marks what needs attention. Healthy values stay plain, headings are bold, and details are dimmed.
* Messages use sentence case. CPU, RAM, GPU, and VRAM are always capitals.
* Every failed check prints the exact command that fixes it.

## Writing Documentation

* Use plain words and short sentences. Explain a technical term the first time it appears.
* Use bullets, numbered steps, and tables instead of paragraphs.
* Capitalize every word in titles.
* Never use em dashes, or hyphens in ordinary text. Write "long lived", not the joined form. Hyphens are fine in code.
* Describe only what is true today. Never present planned work as finished.
* Remove anything that does not help the reader.

## Commits And Pull Requests

### Branches

* Work on a branch. Never commit directly to `master`.

### Commit Messages

* Use this form:

  ```
  type: a short summary in plain words

  An optional body explaining why the change was made.
  ```

| Type | Use For |
| --- | --- |
| `feat` | A new feature or visible change. |
| `fix` | A bug fix. |
| `refactor` | A code change that keeps behavior the same. |
| `docs` | Documentation only. |
| `test` | Tests only. |

* Each commit should make sense alone, build, and pass its tests.

### Pull Requests

* Each pull request describes:

1. What changed, and why.
2. How you tested it, with the exact commands, and whether you used two machines.
3. Anything not yet tested.

* It is ready for review when all checks pass and the documentation is updated.

## Out Of Scope

* Pull requests for these ideas will be closed:

| Idea | Why It Does Not Fit |
| --- | --- |
| Sharing RAM over the network | A network is about a hundred thousand times slower than RAM. |
| Remote desktop or screen streaming | Sunshine and Moonlight already do this well. |
| Copying or mounting build output | `target`, `node_modules`, virtual environments, and caches stay on the Agent. Copying them makes Slingshot too slow. |
| Syncing environment files as source | `.env`, `.env.*`, `*.env`, and `.envrc` often hold secrets. They are never synced. |
| Requiring an account or VPN | A VPN is used when present, never required. iroh needs no account and no open router port. |
| Custom `ssh`, `rsync`, `tmux`, or relay servers | Slingshot wraps these tools and never replaces them. |

## Reporting A Problem

1. Check [TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) first.
2. If your problem is not there, open an issue with:
   * The exact command you ran.
   * What you expected.
   * What happened, with the full output.
   * Each machine's operating system, and how they connect: same network, tailnet, or different networks.
   * The output of `slingshot health`, if the Agent can be reached.
