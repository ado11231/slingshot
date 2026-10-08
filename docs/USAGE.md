# Usage

* Every Slingshot command, what it does, and its options, in the order you will need them.
* If a command prints an error, look it up in [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

## Contents

1. [Before You Begin](#before-you-begin)
2. [Install](#install)
3. [Set Up](#set-up)
4. [Run Work](#run-work)
5. [Keep Files In Step](#keep-files-in-step)
6. [Environment Files](#environment-files)
7. [Check On The Agent](#check-on-the-agent)
8. [Manage Jobs](#manage-jobs)
9. [The Menu Bar App](#the-menu-bar-app)
10. [Work With More Than One Agent](#work-with-more-than-one-agent)
11. [Project Settings](#project-settings)
12. [What Slingshot Copies](#what-slingshot-copies)
13. [Remove Slingshot](#remove-slingshot)

## Before You Begin

* The **Client** is the machine you work on, such as your laptop.
* The **Agent** is the powerful machine that does the work.
* A **project** is a folder inside a Git repository, or a folder with a `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements.txt`, or `slingshot.toml` file.
* A **job** is one run or one session on the Agent.
* Each machine needs these tools. Slingshot tells you if one is missing and prints the command to install it.

| Machine | Needs |
| --- | --- |
| Client | `rsync` |
| Agent | An ssh server, `rsync`, and `tmux` |

## Install

1. Install what Slingshot needs, and Rust, which builds it:

| System | Commands |
| --- | --- |
| Arch Linux | `sudo pacman -S --needed base-devel rsync tmux openssh rustup`<br>`rustup default stable`<br>`sudo systemctl enable --now sshd` |
| Ubuntu or Debian | `sudo apt install rsync tmux openssh-server curl build-essential`<br>`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`<br>`sudo systemctl enable --now ssh` |
| Fedora | `sudo dnf install rsync tmux openssh-server gcc`<br>`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`<br>`sudo systemctl enable --now sshd` |
| macOS | `xcode-select --install`<br>`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |

* On a Linux Client, skip `tmux` and the ssh server. They are only needed on the Agent.
* A Mac as the Agent also needs `brew install tmux`, and Remote Login turned on in System Settings, then General, then Sharing.

2. Install Slingshot on both machines:

   ```sh
   cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli
   ```

3. If `slingshot` is then not found, add Rust's folder to your PATH, then open a new terminal. Use `~/.zshrc` instead if your shell is zsh.

   ```sh
   echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
   ```

* Slingshot is built from GitHub until it is published to crates.io.
* To update later, run the same command again on both machines. Both must run the same version. If the Agent starts Slingshot by itself, restart it there with `systemctl --user restart slingshot.service` on Linux, or by logging out and in on macOS.
* This also installs `sling`, a short name for `slingshot`. `sling run cargo build` and `slingshot run cargo build` do the same thing. These docs always write `slingshot`.

## Set Up

### Start The Agent

1. On the powerful machine, run:

   ```sh
   slingshot start
   ```

2. Slingshot checks the machine.
3. The first time, it offers to start by itself, so the Agent stays reachable after you close the terminal and after a restart. It shows the file it writes and the commands it runs, then asks once. See [Start By Itself](#start-by-itself).
4. Then it prints a pairing code. While Slingshot runs, it keeps the machine awake.

* The code works once, expires after 10 minutes, and stops working after 3 wrong tries. Press Enter for a new one.
* The code never crosses the network, so someone watching the network cannot use it.
* Once a Client is linked, `slingshot start` lists the linked Clients instead of a code. Press Enter when you want a code to link another machine.
* Running `slingshot start` again while it already runs on the same account shows the running Agent and lets you make a code. Ctrl C there only leaves. The first one keeps running.

| Option | Effect |
| --- | --- |
| `--name <name>` | The name the Agent is shown as. The default is its hostname. |
| `--port <port>` | The pairing port. The default is `7433`. If it is taken, such as by another account's `slingshot start`, the next free port is used, and the link line says which. |
| `--boot` | Set up starting by itself without asking, such as after answering no. |
| `--remove` | Stop Slingshot starting by itself, and stop the running service. |

### Start By Itself

* Answer yes when `slingshot start` asks, or run `slingshot start --boot` later.

| Agent | What Slingshot Writes | When It Starts | Logs |
| --- | --- | --- | --- |
| Linux | `~/.config/systemd/user/slingshot.service`, a systemd user service, plus `loginctl enable-linger` | At boot, before anyone logs in | `journalctl --user -u slingshot.service` |
| macOS | `~/Library/LaunchAgents/dev.slingshot.agent.plist`, a launchd agent | At login | `~/Library/Application Support/slingshot/agent/daemon.log` |

* The service never makes a pairing code. To link another machine, run `slingshot start` at the Agent and press Enter. Ctrl C there leaves the service running.
* The service keeps the PATH of the terminal it was set up from, so it finds the same tools.
* If `enable-linger` is refused, Slingshot starts at login instead of at boot, and prints the `sudo` command that fixes it.
* To stop it starting by itself, run `slingshot start --remove` on the Agent.

### Link The Client

1. On the machine you work on, run `slingshot link` with the code:

   ```sh
   slingshot link 192.168.1.9:7433:K7QW9ZR2
   ```

2. Both machines must be on the same network or tailnet for this step. A tailnet is a private network made by a VPN such as Tailscale.
3. Once linked, the Client reaches the Agent from any network.
4. Slingshot then offers to set up your tools on the Agent. See the next section.

* Its output comes in sections: Pairing, Tools, Sign in, Menu bar, and Ready. A section with nothing to do is left out.

* Each machine links under its own name, such as `MacBook-Pro-3f9c2a`, so two machines with the same hostname can use one Agent without replacing each other.
* Linking an Agent again keeps the name it had. Machines linked before September 25, 2026 go by their hostname alone. To get a unique name, run `slingshot unlink`, then link again.

| Option | Effect |
| --- | --- |
| `--name <name>` | Save the Agent under a name of your choice. |

### Set Up Tools On The Agent

* Work runs on the Agent, so the tools you use must be installed there too.
* At the end of `slingshot link`, and any time you run this command, Slingshot compares both machines:

  ```sh
  slingshot tools
  ```

* It offers each tool the Client has and the Agent lacks, plus what the current project needs:

| Tool | How It Is Installed On The Agent |
| --- | --- |
| Git | The package manager |
| Docker | The package manager, then the service is started and you are added to the `docker` group |
| Node and npm | The package manager |
| Python | The package manager |
| Rust | The official `rustup` installer |
| Claude Code | The official installer |
| Codex | `npm` |

* Slingshot then:

1. Lists every command it will run on the Agent.
2. Asks once: `Install them on archbox now? [Y/n]`.
3. Runs commands that need `sudo` in your terminal, so you can type your password. The rest, such as the Claude Code installer, run without one, and their output is shown dimmed under the step, so an installer cannot clear your screen.
4. Checks the Agent again and says which tools were installed.
5. Starts the sign in for Claude Code and Codex, unless they are already signed in. Your accounts are never copied from the Client.

* For Codex, open the link it prints in a browser on the Client. Slingshot forwards port 1455 so the sign in can finish.
* If Claude Code or Codex is already on the Agent but signed out, Slingshot names it and offers to sign in, after one yes.
* On an Agent account that cannot use `sudo`, Slingshot installs only what that account can, and lists the rest under "Ask an admin to run".
* Without a terminal, such as in a script, it only prints the list.
* Many installers, such as the one for Claude Code, put programs in `~/.local/bin` or `~/.cargo/bin` on the Agent. Slingshot adds both folders to the PATH for the tool check, sign ins, `slingshot run`, and sessions, so you never edit a startup file for them.
* Slingshot does not match versions. Files such as `.nvmrc` or `rust-toolchain.toml` in your project still decide exact versions.

## Run Work

### Run A Single Command

* From inside your project, put `slingshot run` in front of any command:

  ```sh
  slingshot run cargo build
  ```

* Slingshot then:

1. Copies the files you changed to the Agent.
2. Warns you if the Agent is low on memory or disk space.
3. Runs the command in the same folder of the Agent's copy, reached at `~/Slingshot/<project>` on the Agent, and streams the output back. Docker Compose names the project after it.

* A line shows where it runs: the Agent, the connection path, the project, and where build output goes.

  ```
  ▶ Running on archbox via local network · app · target → Agent disk
  ```

* Typing, colors, and Ctrl C work as they do locally. At the end, Slingshot shows the time taken and the exit code.
* When the Agent may not know your terminal's type, such as Kitty's `xterm-kitty`, Slingshot sends it as `xterm-256color`, so nothing needs installing on the Agent.
* Slingshot's own options go before `run`. Everything after `run` belongs to your command:

  ```sh
  slingshot --agent archbox run cargo test --release
  ```

* Outside a project, `slingshot run` copies nothing and runs in your home folder on the Agent.
* A run stops if your connection drops. For work that must keep going, use a session.

### Open A Session

* Open a lasting terminal on the Agent:

  ```sh
  slingshot attach
  ```

* Where it opens depends on where you run it:

| Run It From | You Get |
| --- | --- |
| Inside a project | A session in the Agent's copy of the project, at `~/Slingshot/<project>` on the Agent. |
| Anywhere else | A session in the Agent's home folder. Nothing is copied. |

* **Every `attach` in a project copies your latest edits first**, including when you return to a session that is already running.
* If that copy hits a conflict, Slingshot warns you and still opens the session.
* **Leaving a project session brings the Agent's edits back**, such as a coding agent's work, as `slingshot sync --pull` does. A conflict is a warning, and nothing is overwritten.
* Losing the connection does not bring edits back, because the session is still running and a file may be half written. Run `slingshot sync --pull` once the work is done.
* The session keeps running when you disconnect or lose your network. Run `slingshot attach` again to return to it.
* Each project has one session, and the home folder has one session.
* Inside a session:

1. Scroll back through output with your trackpad or mouse. Scrolling never moves the cursor. Press `q` to return to the bottom.
2. Drag to select text. It stays highlighted in blue and is copied to your clipboard. Click anywhere, or press Esc or `q`, to clear it and type again.
3. The bar at the bottom shows the Agent, the connection path, and the project, such as `▶ archbox via local network · app`.
4. Returning to a session that waits at a prompt starts with a clear screen. Earlier output is still there when you scroll up.
5. To clear the screen and all earlier output, press Ctrl B, then K.
6. To leave without stopping it, press Ctrl B, then D.
7. To end it, type `exit`, or run `slingshot stop <id>` from the Client.

* Tools you run in a session, such as Claude Code, Codex, or Docker, must be installed on the Agent. `slingshot tools` installs them.

| Option | Effect |
| --- | --- |
| `[path]` | Open the session for this project folder instead of the current folder. |

## Keep Files In Step

* `slingshot run` and `slingshot attach` copy your changes for you. Use `slingshot sync` to copy files on their own.

| Command | Effect |
| --- | --- |
| `slingshot sync` | Copies your changes to the Agent. |
| `slingshot sync --pull` | Copies changes made on the Agent back to you. |
| `slingshot sync --check` | Shows what a sync would change, without changing anything. |
| `slingshot sync --pull --check` | Shows what a pull would change, without changing anything. |

* Syncing works while a session is open, so you can pull a coding agent's edits back without stopping it.
* **Edits made on only one machine are kept.** A normal sync leaves an edit made only on the Agent alone, and tells you so you can pull it.
* **A conflict stops the sync.** If a file changed differently on both machines, nothing is copied. To fix it:

1. Make each listed file match on both machines, or undo one of the edits.
2. Run the sync again.

## Environment Files

* Files such as `.env` often hold passwords, so Slingshot never copies them with your source.
* Add them on purpose instead. They are stored on the Agent outside the project copy, and placed where you choose when your command runs.

```sh
slingshot env add --file .env --target .env
slingshot env list
slingshot env remove .env
```

| Option For `env add` | Effect |
| --- | --- |
| `--file <file>` | The file on this machine to upload. Its contents are never printed. |
| `--target <path>` | Where it appears in the Agent's copy, such as `.env` or `api/.env.local`. |
| `--replace` | Replace a file already stored at that target. |

* The target must be an environment file name, such as `.env`, `.env.local`, `prod.env`, or `.envrc`.
* Each file can be up to 1 MiB.

## Check On The Agent

| Command | Shows |
| --- | --- |
| `slingshot health` | The Agent's system and processor, then current CPU, RAM, GPU, VRAM, disk, and project space. |
| `slingshot health --watch` | The same, refreshed every two seconds, with running jobs listed underneath. Press Q or Ctrl C to leave. |

* The system and processor were saved when linking, so they need no extra request.
* Values turn yellow or red only when they need attention.

## Manage Jobs

| Command | Effect |
| --- | --- |
| `slingshot ps` | Lists running jobs. |
| `slingshot ps --all` | Also lists the last 100 finished jobs and how each ended. |
| `slingshot stop <id>` | Stops a job. The first few characters of the ID from `slingshot ps` are enough. |

* Stopping asks the job to finish, then forces it after five seconds.
* Your files and build output are kept.
* A finished run shows how it ended: `Completed`, `Failed` with its exit code, `Stopped` by `slingshot stop`, `Interrupted` by Ctrl C, or `Lost connection` when the connection dropped.

## The Menu Bar App

### Install It

* On a Mac, `slingshot link` offers the app when it finishes, and asks once.
* To add it later, run:

  ```sh
  slingshot menubar
  ```

* The first time, Slingshot builds the app and installs it in `~/Applications`. This takes about a minute and needs the Xcode command line tools.
* The app always starts when you log in.
* After you update Slingshot, the app switches to the new version by itself. If the app itself changed, the panel asks you to run `slingshot menubar`, which rebuilds it.

### Remove It

```sh
slingshot menubar --remove
```

* It quits the app, stops it starting at login, and deletes the app, its build folder, and its settings. It shows the list and asks first.

### What It Shows

* Click the icon to open the panel. It shows the Agent, how it is connected, and four sections:

| Section | Shows |
| --- | --- |
| CPU | How busy the processor is. |
| RAM | Memory in use, out of the total. |
| GPU | How busy the graphics card is, its temperature, and its memory (VRAM). |
| Workspace | Free space for projects and build output. |

* When the Agent cannot be reached, the panel shows:

1. Why, in plain words.
2. The command that fixes it.
3. A Try again button.
4. When it last answered.

### Notifications

* The app notifies you when:

1. A run of 10 seconds or more finishes or fails.
2. A run or session is interrupted.
3. The Agent goes offline, and when it comes back.
4. The Agent is low on memory or disk space, or its GPU is running hot.

* Each warning is sent once, and again only after the problem clears.
* If none appear, turn them on in System Settings, then Notifications, then Slingshot.

## Work With More Than One Agent

* Choose an Agent for any command with `--agent`:

  ```sh
  slingshot --agent archbox health
  ```

* With one Agent linked, Slingshot uses it automatically.
* With several, pass `--agent` each time, or set a default in Slingshot's `config.toml`:

  ```toml
  default = "archbox"
  ```

* If several are linked and none is the default, Slingshot tells you where `config.toml` is.

## Project Settings

* A `slingshot.toml` file in the project's top folder is optional. Common Rust, Node, and Python projects need none.
* It supports one setting, extra files to leave out of the copy:

  ```toml
  [sync]
  exclude = ["data/", "*.mp4", "scratch/"]
  ```

* Each pattern works like a `.gitignore` line, matched from the top of the project.
* Patterns that begin with `!` are not allowed.

## What Slingshot Copies

* Slingshot copies your source files only. It never copies:

1. Anything your `.gitignore` excludes.
2. Version control folders such as `.git`.
3. Build output, such as `target`, `node_modules`, `.venv`, and caches.
4. Environment files: `.env`, `.env.*`, `*.env`, and `.envrc`.
5. Anything under `sync.exclude` in `slingshot.toml`.

* Build output stays in separate storage on the Agent:

| Project | Where Build Output Goes |
| --- | --- |
| Rust | `target` is written to Agent storage. |
| Node | `node_modules` links to Agent storage. |
| Python | `.venv` stays in the Agent's copy and is never copied back. The pip cache is kept in Agent storage. |

## Remove Slingshot

1. Stop any running jobs with `slingshot stop <id>`. `unlink` will not run while a job is active.
2. On the Client, run:

   ```sh
   slingshot unlink
   ```

3. This removes this machine's key and environment files from the Agent, and forgets the Agent.

* Project copies on the Agent are kept.
* To stop the Agent itself, run `slingshot start --remove` on the Agent if it starts by itself. Otherwise press Ctrl C where `slingshot start` is running.
