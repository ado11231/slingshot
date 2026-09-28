# Usage

* Every Slingshot command, what it does, and its options, in the order you will need them.
* If a command prints an error, look it up in [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

## Contents

1. [Before You Begin](#before-you-begin)
2. [Set Up](#set-up)
3. [Run Work](#run-work)
4. [Keep Files In Step](#keep-files-in-step)
5. [Environment Files](#environment-files)
6. [Check On The Agent](#check-on-the-agent)
7. [Manage Jobs](#manage-jobs)
8. [The Menu Bar App](#the-menu-bar-app)
9. [Work With More Than One Agent](#work-with-more-than-one-agent)
10. [Project Settings](#project-settings)
11. [What Slingshot Copies](#what-slingshot-copies)
12. [Remove Slingshot](#remove-slingshot)

## Before You Begin

* The **Client** is the machine you work on, such as your laptop.
* The **Agent** is the powerful machine that does the work. It is also called the box.
* A **project** is a folder inside a Git repository, or a folder with a `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements.txt`, or `slingshot.toml` file.
* A **job** is one run or one session on the Agent.
* Each machine needs these tools. Slingshot tells you if one is missing and prints the command to install it.

| Machine | Needs |
| --- | --- |
| Client | `rsync` |
| Agent | An ssh server, `rsync`, and `tmux` |

## Set Up

### Start The Agent

1. On the powerful machine, run:

   ```sh
   slingshot start
   ```

2. Slingshot checks the machine, then prints a pairing code.
3. Leave it running. While it runs, it keeps the machine awake.

* The code works once, expires after 10 minutes, and stops working after 3 wrong tries. Press Enter for a new one.
* The code never crosses the network, so someone watching the network cannot use it.

| Option | Effect |
| --- | --- |
| `--name <name>` | The name the Agent is shown as. The default is its hostname. |
| `--port <port>` | The pairing port. The default is `7433`. |

### Link The Client

1. On the machine you work on, run `slingshot link` with the code:

   ```sh
   slingshot link 192.168.1.9:7433:K7QW9ZR2
   ```

2. Both machines must be on the same network or tailnet for this step. A tailnet is a private network made by a VPN such as Tailscale.
3. Once linked, the Client reaches the Agent from any network.
4. Slingshot then offers to set up your tools on the Agent. See the next section.

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
3. Runs them in your terminal, so you can type your password when `sudo` asks.
4. Checks the Agent again and says which tools were installed.
5. Starts the sign in for Claude Code and Codex. Your accounts are never copied from the Client.

* For Codex, open the link it prints in a browser on the Client. Slingshot forwards port 1455 so the sign in can finish.
* Without a terminal, such as in a script, it only prints the list.
* Many installers, such as the one for Claude Code, put programs in `~/.local/bin` or `~/.cargo/bin` on the Agent. Slingshot adds both folders to the PATH for the tool check, sign ins, and `slingshot run`, so you never edit a startup file for them. Sessions read your startup files as usual.
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
3. Runs the command in the same folder of the Agent's copy, and streams the output back.

* A line shows where it runs: the Agent, the connection path, the project, and where build output goes.

  ```
  ▶ Running on archbox via local network · app · target → Agent disk
  ```

* Typing, colors, and Ctrl C work as they do locally. At the end, Slingshot shows the time taken and the exit code.
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
* The session keeps running when you disconnect or lose your network. Run `slingshot attach` again to return to it.
* Each project has one session, and the home folder has one session.
* Inside a session:

1. Scroll back through output with your trackpad or mouse.
2. The bar at the bottom shows the Agent and the project, such as `▶ archbox · app`.
3. To leave without stopping it, press Ctrl B, then D.
4. To end it, type `exit`, or run `slingshot stop <id>` from the Client.

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
| `slingshot info` | Hardware: CPU, RAM, GPU, disk, and tools. Saved when linking, so it is instant. Add `--refresh` to ask again. |
| `slingshot health` | Current CPU, RAM, GPU, VRAM, disk, and project space. |
| `slingshot health --watch` | The same, refreshed every two seconds. Press Q or Ctrl C to leave. |
| `slingshot top` | Live resources, with running jobs listed underneath. |

* Values turn yellow or red only when they need attention.

## Manage Jobs

| Command | Effect |
| --- | --- |
| `slingshot ps` | Lists running jobs. |
| `slingshot ps --all` | Also lists the last 100 finished jobs and how each ended. |
| `slingshot stop <id>` | Stops a job. The first few characters of the ID from `slingshot ps` are enough. |

* Stopping asks the job to finish, then forces it after five seconds.
* Your files and build output are kept.

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
| Python | `.venv` links to Agent storage, and the pip cache is kept there too. |

## Remove Slingshot

1. Stop any running jobs with `slingshot stop <id>`. `unlink` will not run while a job is active.
2. On the Client, run:

   ```sh
   slingshot unlink
   ```

3. This removes this machine's key and environment files from the Agent, and forgets the Agent.

* Project copies on the Agent are kept.
* To stop the Agent itself, press Ctrl C where `slingshot start` is running.
