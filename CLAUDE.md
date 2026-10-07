# CLAUDE.md

* Rules for AI coding agents in this repository, including Claude Code, Codex, and any other tool.
* These rules take priority over your defaults. Read the whole file before any change, and follow it exactly.
* AI contributions are welcome only when every line is understood, checked, and justified. Code that only looks finished is worse than no code.
* "Must" and "never" are absolute. If a request would break a rule, stop, name the rule, and propose an alternative. Never bend a rule quietly.

## Contents

1. [Accountability](#1-accountability)
2. [Read First](#2-read-first)
3. [What Slingshot Is](#3-what-slingshot-is)
4. [Rules That Are Never Broken](#4-rules-that-are-never-broken)
5. [Out Of Scope](#5-out-of-scope)
6. [Before Writing Code](#6-before-writing-code)
7. [Writing Code](#7-writing-code)
8. [Output Style](#8-output-style)
9. [Verification](#9-verification)
10. [Documentation](#10-documentation)
11. [Git](#11-git)
12. [When To Stop And Ask](#12-when-to-stop-and-ask)
13. [Reporting Your Work](#13-reporting-your-work)
14. [Final Checklist](#14-final-checklist)
15. [Current State](#15-current-state)

---

## 1. Accountability

* **Understand every line.** You must be able to explain what each line does and why it is needed. If you cannot, do not write it.
* **Never invent.** Never assume a function, type, method, crate, flag, file, or path exists. Confirm it in the source, the crate's documentation, or by running it.
* **Never fabricate results.** Never report a test as passing, a command as working, or output as seen unless you ran it and read the result.
* **Read what you run.** Read the full output of every command before acting on it or describing it.
* **Back up claims.** When describing existing code, name the file, and the line when it matters.
* **State what you do not know.** Plain uncertainty is useful. Confidence without evidence is not.

---

## 2. Read First

* Before your first change in a session, read:

| Document | Why |
| --- | --- |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How the parts fit together, and what every file does and depends on. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Standards for all contributors. Every rule there applies to you too. |
| [docs/ROADMAP.md](docs/ROADMAP.md) | What works, what was tested, and the known limitations. |
| [docs/USAGE.md](docs/USAGE.md) and [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | What users see and read. |

* If the code and a document disagree, the code is the truth. Fix the document in the same change.

---

## 3. What Slingshot Is

* Slingshot lets a light machine use the CPU, RAM, and GPU of a powerful one.
* The user keeps their own editor, terminal, and browser. Builds, servers, databases, containers, coding agents, and small AI models run on the powerful machine.
* It must feel local. There is no virtual machine and no remote desktop.
* **Goal:** a public open source tool a stranger can install and set up in minutes. Setup friction is a bug.
* **Name:** Slingshot. The command is `slingshot`. It was called borrow until September 22, 2026.
* **Languages:** Rust, except the macOS menu bar app, which is SwiftUI.
* **Roles:** the **Client** is the machine the user works on. The **Agent** is the machine with the resources.

---

## 4. Rules That Are Never Broken

1. **The Client stays light.** Never move heavy work onto the Client. If a change makes the Client do real work, the change is wrong.
2. **Always state where work runs.** Every remote command prints its location, such as `▶ Running on archbox via tailnet`.
3. **Wrap existing tools. Never rebuild them.** Slingshot wraps `ssh`, `rsync`, `tmux`, `docker`, `ollama`, `nvidia-smi`, and the `iroh` library. Never write your own ssh, file transfer, terminal multiplexer, container engine, inference engine, relay server, or NAT traversal.
4. **Keep setup simple.** One program per machine, one pairing step, one command to use. Reject designs that add setup steps.
5. **Stay platform neutral.** Describe everything as Client and Agent, never as Mac and Linux. Mac to Linux and Linux to Linux must both work. Only the menu bar app may be macOS specific.
6. **Design for a stranger.** Anything easy only on the author's machine becomes a setup check or an installer step. Every failed check prints the exact fix. Detect and instruct. Never install without asking: show exactly what will run, then ask once.
7. **Require nothing a stranger lacks.** A VPN such as Tailscale is used when present, never required. iroh connects from anywhere with no account, no server, and no open router port.

---

## 5. Out Of Scope

* Refuse these, and explain why:

| Request | Reason |
| --- | --- |
| Sharing RAM over the network | A network is about a hundred thousand times slower than RAM. |
| Remote desktop or screen streaming | Sunshine and Moonlight already do this. |
| Copying or mounting build output | `target/`, `node_modules/`, virtual environments, and caches stay in separate Agent storage. This is the most important performance rule. |
| Treating environment files as source | `.env`, `.env.*`, `*.env`, and `.envrc` never enter a sync, and their contents never appear in arguments. |
| Assuming a package manager | The main test Agent runs Arch Linux, which uses `pacman`, not `apt`. Detect and instruct. |
| Fixed system paths in shared code | Never write `/Users/...` or `/home/...`. Use the `directories` crate. |
| New operations on the pairing port | The TCP port only pairs. Every other request uses `slingshot internal-control` over ssh. |
| A custom way to run commands | ssh carries all work. `run`, `attach`, and `rsync` always go through it. |

---

## 6. Before Writing Code

1. **Understand the request.** If it is unclear or has more than one meaning, ask. Never guess about anything that changes behavior, deletes data, or affects security.
2. **Read the code you will change, and its callers.** Find both in the [file reference](docs/ARCHITECTURE.md#file-reference). Never edit a file you have not read.
3. **Look for existing helpers.** Search before writing anything new:

   | Need | Existing Helper |
   | --- | --- |
   | Printing output | `presentation::{success, warning, detail, row, capacity, plural}`, `Style`, `step::start` |
   | Talking to the Agent | `client::Control`, `client::request`, `client::fetch`, `route::resolve` |
   | Storing files | `storage::{data_dir, write_json, read_json, lock, now}` |
   | Setup checks | `preflight::tool_check`, `preflight::report` |

4. **Choose the right crate.**
   * Rules both machines apply go in `slingshot-core`.
   * Code only the Agent runs goes in `slingshot-agent`.
   * Commands and Client code go in `slingshot-cli`.
   * Dependencies flow one way: `slingshot-cli` uses the other two, `slingshot-agent` uses `slingshot-core`, and `slingshot-core` uses neither.
5. **Plan the smallest change** that solves the problem and keeps everything working end to end.

---

## 7. Writing Code

### Scope

* Do exactly what was asked, and nothing more.
* Never refactor, rename, or reformat code you were not asked to change.
* Never add an abstraction, trait, generic, or setting for a single use.
* Never add features or options for possible future needs.
* Remove everything you added that ended up unused: functions, imports, fields, and files.
* Never leave debug output, test values, or temporary files behind.

### Size

* If a change passes about 300 changed lines, or touches more than five files the request did not mention, stop, describe the plan, and ask.
* Split large work into steps that each build, pass tests, and can be reviewed alone.

### Rust

* Match the surrounding code's names, structure, and idioms.
* Use plain English names. Never names like `mgr`, `ctx2`, `tmp_data`, or `handle_stuff`.
* Errors use `anyhow`. Every visible error says what went wrong and what to do, in one sentence, with the exact command when there is one.
* A failing user command is not a Slingshot error. Report its exit code faithfully.
* Never use `unwrap()` outside tests. Use `?`, or `expect("reason this cannot fail")` for true invariants only.
* Never use `unsafe` unless existing code already does the same thing, and explain why.
* Never add `#[allow(...)]` to silence a warning. Fix the cause, or ask.
* Use `tokio` for async code. Never block inside async code; use `spawn_blocking`.
* Use `tracing` for logs, and the presentation helpers for user output. Never raw escape codes.
* Use `clap` for arguments, and `serde`, `toml`, and `serde_json` for settings and messages.
* Hardware and health data come from `sysinfo` and `nvidia-smi`.

### Comments

* Add a short comment above an item only when its purpose is not obvious. Explain why.
* Never put comments inside function bodies.
* Never write a comment that repeats the code.
* Never leave commented out code, or `TODO`, `FIXME`, or `XXX` notes.
* Never use em dashes in code, comments, or messages.

### Formats And Versions

* **Control messages:** any change to a `Request` or `Response` shape in `slingshot-core/src/control.rs` raises `control::VERSION`, currently 11.
* **Menu bar lines:** if a field in `slingshot-cli/src/watch/event.rs` changes meaning or is removed, raise `watch::event::VERSION` and change `slingshot-cli/menubar/Sources/Model.swift` in the same commit.
* **Pairing messages:** a change to the pairing messages in `slingshot-core/src/protocol.rs` means both machines must update before the next link. A change to `Specs` or `Health`, which are saved or sent after pairing, forces every user to link again. Say which one explicitly.

### Security

* Never weaken these protections:

1. Pairing codes never cross the network. They work once, expire, and are burned after 3 wrong tries. Both sides prove they know the code before anything is trusted.
2. The daemon listens only on the machine itself and the local network, never on `0.0.0.0`.
3. Installed keys are named so `unlink` can remove them.
4. Arguments sent to the Agent are quoted one by one with `shell-words`, never joined into one shell command.
5. Control messages carry a version, a size limit, and a time limit.
6. The Agent's user account is the security boundary.
7. Processes are never stopped by ID alone. Their start time is checked too.
8. The Agent accepts iroh connections only from paired Clients, and passes them only to its own ssh server.
9. Secrets and environment file contents never appear in arguments, logs, or output.

### The Menu Bar App

* Swift only displays information and posts notifications. All thresholds, rules, and wording live in Rust, in `crates/slingshot-cli/src/watch/`.
* Confirm an SF Symbol name exists before using it. A wrong name draws nothing and reports no error.
* macOS only Rust code sits behind `#[cfg(target_os = "macos")]`, with a clear message on other systems.

### Dependencies

* Never add a crate without asking. Name it, explain why, and say which existing tool could be wrapped instead.
* Once approved, look up its current version on crates.io. Never assume a version.
* Shared versions go in the root `Cargo.toml`, under `[workspace.dependencies]`.

### Risky Actions

* Never do these without explicit permission in the current conversation:

1. Delete files you did not create, or run `rm -rf`.
2. Run `git reset --hard`, rewrite history, or force push.
3. Change or delete keys, configuration, or user data on either machine.
4. Edit anything outside this repository, such as `~/.ssh/config`.
5. Install software or change system settings.

---

## 8. Output Style

* All output goes through `slingshot-core/src/presentation.rs` and `slingshot-core/src/step.rs`.
* Slow work is a step: a spinner in a terminal, then a line such as `✓ Synced 3 files  0.4s`.
* Status symbols: `✓`, `!`, `✗`, and `▶`.
* Color marks only what needs attention: symbols, the Agent and path in `▶` lines, and warning or failing values. Healthy values stay plain, headings are bold, and details are dimmed.
* Messages use sentence case. CPU, RAM, GPU, and VRAM are always capitals.
* `--color auto|always|never` controls color. Slingshot's options come before `run`. Everything after `run` belongs to the user's command.

---

## 9. Verification

* A change is not finished until all three checks pass with zero warnings:

  ```sh
  cargo fmt --check
  cargo clippy --workspace --all-targets
  cargo test --workspace
  ```

* If you changed the menu bar app, also run:

  ```sh
  swift build -c release --package-path crates/slingshot-cli/menubar
  ```

* Every rule or decision you add or change gets a unit test in the same file's `tests` module.
* Run the changed command at least once, for real, and read its output.
* If a change needs two machines and you cannot reach them, say so. Never imply you tested it.
* Never weaken, skip, or delete a test to make it pass. If a test is wrong, explain why before changing it.

---

## 10. Documentation

* **Where changes go:**

| If You | Update |
| --- | --- |
| Add or move a file | The [file reference](docs/ARCHITECTURE.md#file-reference), with its purpose, what it uses, and what uses it |
| Change a command or option | [USAGE.md](docs/USAGE.md) |
| Add or change an error message | [TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) |
| Change what works | [ROADMAP.md](docs/ROADMAP.md), with the date, recording only what was tested |

* **How to write:**

1. Use plain words and short sentences. Explain a technical term the first time it appears.
2. Use bullets, numbered steps, and tables. No paragraphs.
3. Capitalize every word in titles.
4. Never use em dashes, or hyphens in ordinary text. Write "long lived", not the joined form. Hyphens are fine in code.
5. Never use empty words such as "robust", "seamless", "leverage", "comprehensive", or "simply".
6. Remove anything that does not help the reader.
7. Never describe planned work as finished.

---

## 11. Git

* Commit only when asked. Push only when asked. Never force push.
* Work on a branch, never directly on `master`.
* Commit messages follow `type: summary`, with type `feat`, `fix`, `refactor`, `docs`, or `test`. The body explains why.
* Make small commits that each build and pass tests.
* **No AI attribution.** Never add `Co-Authored-By` lines, "Generated with" lines, or tool names. Commits belong to the person who asked for them.
* Never commit secrets, build output, or anything in `crates/slingshot-cli/menubar/.build/`.

---

## 12. When To Stop And Ask

* Stop and ask before continuing when:

1. The request conflicts with any rule in this file.
2. You need a new dependency.
3. You would change a message format or a stored file format.
4. You would delete or move user data, keys, or configuration.
5. You would change anything covered by the security rules.
6. The change passes the size limits in section 7.
7. You are unsure what the user wants.

---

## 13. Reporting Your Work

* Report in this order, briefly and factually:

1. **What changed**, file by file.
2. **What you verified**, with the commands you ran and their results.
3. **What you did not verify**, and why.
4. **What the user must do**, such as linking again after a format change.

* Never describe work as done if part of it is not. Never hide a failure inside a longer summary.

---

## 14. Final Checklist

* Every line must be true before you report that you are finished:

- [ ] I changed only what was asked, within the size limits.
- [ ] I read every file I edited, and the code that calls it.
- [ ] Every function, flag, and path I used exists. I checked.
- [ ] I reused existing helpers instead of writing new ones.
- [ ] New code matches the style around it.
- [ ] No unused code, debug output, commented out code, or TODO notes remain.
- [ ] Every visible error says how to fix it.
- [ ] Formatting, Clippy, and tests pass with zero warnings.
- [ ] I ran the changed command and read the output, or I clearly said I could not.
- [ ] Format versions are raised wherever a shape changed.
- [ ] Documentation is updated, and claims only what was tested.
- [ ] My report separates what I verified from what I did not.

---

## 15. Current State

* **Phases 1 and 3 are complete.** Phase 4, reaching the Agent from any network, is in final testing. Phase 5 is in progress: the menu bar app and notifications are built, and automatic port forwarding is not.
* **Next:** the cleanup listed in issue #51, then port forwarding (#9), then local models (#20, #38 to #40).
* **All tests, formatting, and Clippy pass on `master`.** CI checks every pull request on Linux and macOS.
* **The rename from borrow has no migration.** Both machines must run `slingshot link` again after updating.
* Everything under [known limitations](docs/ROADMAP.md#known-limitations) does not work yet. Never describe it as working.
* The full [test record](docs/ROADMAP.md#test-record) is in the roadmap.
