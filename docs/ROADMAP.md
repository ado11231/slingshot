# Roadmap

* What Slingshot does today, what has been tested, and what comes next.
* A feature is listed as complete only after it has run on real machines.
* Last updated September 25, 2026.

## Contents

1. [Status](#status)
2. [How The Work Is Organized](#how-the-work-is-organized)
3. [Complete](#complete)
4. [In Progress](#in-progress)
5. [Planned](#planned)
6. [Known Limitations](#known-limitations)
7. [Test Record](#test-record)

## Status

| Phase | Goal | Status |
| --- | --- | --- |
| 1 | Pairing, remote commands, and hardware details on a local network | Complete |
| 2 | Project recognition, and keeping build output on the Agent | Replaced by phase 3, which kept both |
| 3 | Source copies, sync, sessions, jobs, environment files, and live health | Complete |
| 4 | Reaching the Agent from any network | Final testing |
| 5 | Menu bar app, notifications, and automatic port forwarding | In progress |
| 6 | AI model tools, pairing across networks, and more platforms | Planned |
| 7 | Installers, packages, and a public release | Planned |

## How The Work Is Organized

* Work is split into phases.
* Each phase leaves Slingshot usable.
* Later phases build on earlier ones, so none are skipped.

## Complete

### Phase 1: Pairing And Remote Commands

* `slingshot start` checks the Agent for an ssh server, `rsync`, and `tmux`, and prints a single use pairing code.
* `slingshot link` installs the Client's key on the Agent. The Client needs no ssh server.
* `slingshot run` runs a command on the Agent with live output, typing, Ctrl C, and the real exit code.
* `slingshot info` and `slingshot health` show the Agent's hardware and current use.

### Phase 2: Mounting The Project On The Agent

* Built on September 7 and 8, 2026, then replaced by phase 3 on September 13.
* The Agent mounted the Client's project folder over SSHFS, a network file system that runs over ssh, and ran work inside it. Build output went to the Agent's own disk.
* It was replaced because:
  * The Agent had to connect back to the Client, so the Client needed its own ssh server. Most people do not run one.
  * Every file operation crossed the network, which made builds slow.
  * A stale mount could hang a build, and file watchers missed changes.
* Phase 3 kept its project recognition for Rust, Node, and Python, and its separate Agent storage for build output.
* The last of the mount code was removed on September 20, 2026.

### Phase 3: Source Copies, Sessions, And Live Status

* The Agent keeps its own copy of each project, kept in step by three way sync. Conflicts stop the sync, so no edit is lost.
* Build output for Rust, Node, and Python stays in separate Agent storage.
* `slingshot attach` opens or returns to one lasting session per project.
* `slingshot sync` pushes changes, pulls them with `--pull`, and previews with `--check`.
* `slingshot env` stores environment files on the Agent, apart from the source copy.
* `slingshot ps` and `slingshot stop` list and stop jobs.
* `slingshot health --watch` and `slingshot top` refresh live.
* `slingshot unlink` removes this Client's key and environment files from the Agent.

## In Progress

### Phase 4: Reaching The Agent From Any Network

* **Built:**

1. The Client tries the local network, then the tailnet, then iroh, and uses the first that answers. Every run names its path.
2. iroh connects directly when it can, and through a relay when it cannot. A relay only sees encrypted data.
3. The Agent accepts iroh connections only from paired Clients.
4. `slingshot start` keeps the Agent awake.
5. Error messages tell an Agent that is off apart from a Client that is no longer paired.

* **Remaining:**

1. Test a connection lost for more than 3 minutes during a build over iroh. A drop of about 15 seconds did not interrupt a run.

### Phase 5: Polish

* **Built:**

1. A macOS menu bar app showing CPU, RAM, GPU, VRAM, and workspace space.
2. An offline screen with the cause, the fix, and a Try again button.
3. Notifications for finished, failed, and interrupted runs, the Agent going offline or returning, and low memory, low disk, or a hot GPU.
4. `slingshot attach` from anywhere: a project session, or a home session outside a project.
5. Every `attach` syncs first. `sync`, `sync --pull`, and `run` work while a session is open.
6. Sessions start in the login shell with full color, `UTF-8`, mouse scrolling, a quiet bar, and a readable `~/Slingshot/<project>` path.
7. `slingshot menubar` builds, signs, and installs the app itself, and rebuilds it only when its source changed.
8. `link` ends by offering to install the Client's tools on the Agent, then starts the sign in for Claude Code and Codex. `slingshot tools` repeats it later.
9. The tool check, sign ins, and runs also search `~/.local/bin` and `~/.cargo/bin` on the Agent, where installers such as Claude Code's put programs.
10. The tools step shows one plain command per tool, warns when tools are missing, and numbers each install.
11. A run ended by Ctrl C or `slingshot stop` is reported as stopped, not failed.
12. The menu bar app is always a login item, `link` offers it on a Mac, `slingshot menubar --remove` removes it, and it switches to a newly installed `slingshot` by itself.

* **Remaining:**

1. Automatic port forwarding, so the Agent's port 3000 appears at `localhost:3000` on the Client.
2. Notifications when a server is ready and when a job waits for input.
3. See the new tools output, with a tool actually missing, on real machines.

## Planned

### Phase 6: AI Models, Pairing Anywhere, And Reach

* Support for Ollama and ComfyUI: small language models, embeddings, Whisper, and image generation.
* Pairing across networks, reusing the SPAKE2 exchange so the code stays safe through a relay.
* A setting to use your own iroh relay.
* Stronger support for Linux Clients with Linux Agents.

### Phase 7: Release

* Ready to run programs for macOS and Linux, on Intel and ARM.
* A one line installer that also starts `slingshot start` when the Agent turns on.
* Packages for Arch Linux (AUR) and Homebrew.
* `slingshot doctor`, one command that checks a setup and explains each fix.
* A license, issue templates, and automatic builds and releases.
* **Done when** a newcomer goes from install to a working `slingshot run` in under five minutes, with no help.

## Known Limitations

* `slingshot start` does not start on its own when the Agent restarts.
* The first connection over iroh after the Agent restarts can take about 13 seconds. Later ones take about 1 second.
* Edits a coding agent makes on the Agent stay there until `slingshot sync --pull`.
* The tools step installs Docker only with `pacman`, `apt`, `dnf`, and `zypper`, and Git, Node, and Python only with the package managers it knows.
* File watchers inside sessions, several Clients on one Agent account, and large Node and Python projects are untested. Each Client now links under a unique name, but two Clients have not been linked to one Agent at the same time yet.
* `slingshot.toml` supports only `sync.exclude`. Other settings have no effect.
* Pairing across networks is not supported. Both machines must share a network or tailnet to link.
* The notifications for an Agent coming back online, and the resource warnings, have not been seen on real machines.

## Test Record

* All tests ran between a Mac Client and an Arch Linux Agent named archbox.

### Phase 3: September 13, 2026, Same Network

1. The first `slingshot run cargo build` copied 47 files and finished in 9 seconds. The second started in 1 second.
2. Subfolder runs, exit codes, typing, and Ctrl C behaved as they do locally. The Agent's copy held no `.git` and no `target`.
3. Push, pull, preview, an edit made only on the Agent, and a conflict on both sides all worked. The conflict was refused and nothing changed.
4. Environment files were private to the Agent account, visible to `run`, and never synced.
5. `attach`, leaving, returning, and `stop` all worked.
6. A release build in `attach` kept running while the Client was offline for four minutes, and `attach` returned to it after.
7. Restarting `slingshot start` kept the session. Restarting the Agent marked it interrupted.
8. `unlink` refused while a session was active, then removed the key and environment files after `stop`.

### Phase 4: September 22, 2026

1. `slingshot run` worked over the home network, over the tailnet from a phone hotspot, and over iroh from the hotspot with Tailscale off.
2. Over iroh, the first ssh connection took about 1.5 seconds, and later ones about 0.3 seconds.
3. With `slingshot start` stopped, `run` failed after about 20 seconds with a message naming the cause.
4. The Agent stayed awake while `slingshot start` ran.
5. Over iroh, an unpaired Client was refused and a paired one was accepted.
6. `slingshot sync` and `slingshot attach`, including leaving and returning, worked over iroh.

### Phase 5: September 23, 2026

1. The menu bar app built, installed, registered to start at login, and showed live values from archbox. Starting after a real log out has not been checked.
2. With `slingshot start` stopped, the offline screen named the cause and offered `slingshot start`, then returned to live values on its own.
3. Real runs produced the right notifications: finished, failed, and interrupted, and none for a quick `echo hi`.
4. The offline notification appeared when `slingshot start` was stopped.
5. `slingshot attach` outside a project opened the home session in `~`, with full color, `UTF-8`, mouse scrolling, Claude Code, and `docker ps` working.
6. `slingshot attach` in a project synced first, including into a session already running, and a fresh session showed `~/Slingshot/slingshot` in the prompt.
7. `cargo build` in a project session ran on archbox. The menu bar showed CPU rise, and archbox's fans spun up while the Mac stayed quiet.
8. With the session still open, `slingshot sync --pull` brought a file made on archbox back to the Mac.

### Phase 5: September 24, 2026, Mac Only

* archbox was off, so only the Mac was tested.

1. `slingshot menubar` built, signed, and installed the app in 12 seconds, then opened it.
2. A second `slingshot menubar` skipped the build and took under a second.
3. After a change to the app's Swift source, `slingshot menubar` rebuilt the app on its own.
4. `slingshot tools` with archbox off failed after about 34 seconds with `archbox is not reachable`.

### Safe Pairing: September 25, 2026, Mac Only

* A test Agent ran on the Mac, with a relay in front of it that recorded every byte on the pairing port.

1. Three wrong codes were each refused, the Agent counted them, and the third burned the code. The right code was then refused as used.
2. Pressing Enter in `slingshot start` printed a new code.
3. The recorded traffic held neither the code nor any of the wrong guesses.
4. An old style pairing request was refused with an instruction to update, even with the right code.
5. The new Client linking to an Agent built from the previous version was told to update that Agent.

### Fresh Install: September 25, 2026

* The Mac's Slingshot key, data, app, and program were moved to a backup. A new account, `slingtest`, on archbox played the Agent of someone who had never used Slingshot.
* Both machines followed only the README, from a fresh clone.

1. On `slingtest`, `cargo install` first failed until `rustup default stable` was run, and `~/.cargo/bin` had to be added to the PATH. Both come from Arch Linux's `rustup` package.
2. `slingshot start` passed every check, found the GPU, reached iroh, and printed codes for the local network and the tailnet.
3. A wrong code was refused. The right code paired in 0.1 seconds with a unique Client name.
4. The tools step offered Claude Code and Codex with their commands and asked once. Codex installed with `sudo` and signed in from the Mac's browser through the forwarded port. Claude Code installed into `~/.local/bin` but was reported as missing, because that folder was not on the login PATH.
5. `slingshot run cargo build --release` took 54 seconds on archbox, against 3 minutes 20 seconds on the Mac. The second run took 0.6 seconds.
6. `health`, `ps --all`, `env add`, `stop`, `attach` with leaving and returning, `sync --pull` of a file made in a session, and `slingshot menubar` all worked. The menu bar app built in 12 seconds.
7. On a phone hotspot, runs went through iroh with Tailscale off, and through the tailnet with it on.
8. Wi Fi turned off for about 15 seconds during a 2 minute run over iroh did not interrupt it.
9. After `slingshot start` was restarted, the Mac reconnected over iroh without linking again. The first connection took 13 seconds.
10. `unlink` refused while a session was open, then removed only this Client's key, iroh access, and environment file.
11. Linking the new Client to an Agent running the previous version printed an instruction to update that Agent.

### Fixes From The Fresh Install: September 25, 2026

* Both machines were updated to control protocol 8.

1. `slingshot tools` found Claude Code in `~/.local/bin` and reported that archbox has every tool.
2. `slingshot run claude --version` printed the Claude Code version. The run's PATH began with `~/.local/bin` and `~/.cargo/bin`.
3. A run stopped with `slingshot stop` ended with `! Stopped in 8.4s · exit 130`.
4. archbox's npm folder belongs to root, so the tools step would offer `sudo npm install -g @openai/codex`.

### Menu Bar Always On: September 28, 2026, Mac Only

* archbox had `slingshot start` stopped, so only the Mac was tested.

1. The offline panel showed the cause and the fix, with no greyed old values.
2. `slingshot menubar` rebuilt the app, restarted it, and macOS listed it as an enabled login item.
3. Changing the `slingshot` program file made the app start a new helper within 3 to 9 seconds, in four of five tries. In the first try, right after a rebuild, the old helper was still running 8 seconds later and had exited 20 seconds later. The cause was not found.
4. `slingshot menubar --remove` listed what it would delete, asked, then removed the app, its build folder, and its settings. macOS listed the login item as disabled.
5. Not tested: the offer at the end of `link`, because linking again needs archbox.
