# Troubleshooting

* What each Slingshot error message means, and how to fix it.
* Search this page for the words on your screen. Each entry shows the exact message.

## Contents

1. [Starting The Agent](#starting-the-agent)
2. [Starting By Itself](#starting-by-itself)
3. [Linking](#linking)
4. [Setting Up Tools](#setting-up-tools)
5. [Reaching The Agent](#reaching-the-agent)
6. [Running Commands](#running-commands)
7. [Syncing Files](#syncing-files)
8. [Jobs](#jobs)
9. [GPU](#gpu)
10. [The Menu Bar App](#the-menu-bar-app)
11. [Removing Slingshot](#removing-slingshot)
12. [Getting Help](#getting-help)

## Starting The Agent

### SSH Server Not Running

* **Message:** `SSH server not running`
* **Meaning:** Slingshot sends all work over ssh, so the Agent needs an ssh server. The Client does not.
* **Fix:**

1. On macOS, turn on System Settings, then General, then Sharing, then Remote Login.
2. On Linux, run `sudo systemctl enable --now sshd`.

### Missing Tool

* **Message:** `Tool not installed: rsync` or `Tool not installed: tmux`
* **Meaning:** `rsync` copies files, and `tmux` keeps sessions running. The Agent needs both.
* **Fix:** Run the install command Slingshot printed, then run `slingshot start` again.

### Docker Not Usable

* **Message:** `Docker is installed, but this account cannot use it`
* **Meaning:** This account is not in the `docker` group, so every Docker command fails with "permission denied". It is a warning, so Slingshot still starts.
* **Fix:**

1. Run the command Slingshot printed, such as `sudo usermod -aG docker ado`.
2. Restart the machine. Sessions and Slingshot's service keep the groups they started with, so only a restart gives them the new one.

### Already Running

* **Message:** `Another slingshot start is already running for this account`
* **Meaning:** Two `slingshot start` commands began at the same moment on one account.
* **Fix:** Run `slingshot start` again. It shows the one already running.

### Different Version Already Running

* **Message:** `A different version of Slingshot is already running for this account`
* **Meaning:** The `slingshot start` already running comes from another version of Slingshot, usually one from before an update.
* **Fix:** Stop it with Ctrl C where it runs, then run `slingshot start` again.

### Running Agent Did Not Answer

* **Message:** `Slingshot is running for this account but did not answer`
* **Meaning:** Another `slingshot start` runs on this account, but it did not reply when asked about itself.
* **Fix:** Stop the other `slingshot start` with Ctrl C, then run `slingshot start` again.

### Already Running Under Another Name

* **Message:** `Slingshot is already running here as archbox, not <name>`
* **Meaning:** `--name` asked for a different name than the Agent already running. Linked Clients know the Agent by its current name.
* **Fix:** Run `slingshot start` without `--name`.

### Port In Use

* **Message:** `! Port 7433 is in use, perhaps by another slingshot start, so this one uses 7434`
* **Meaning:** Another program, or another `slingshot start` on this or another account, has the pairing port. Slingshot moved to the next free one.
* **Fix:** Nothing. The printed `slingshot link` line carries the new port.

### No Free Port

* **Message:** `Ports 7433 to 7442 are all in use`
* **Meaning:** Slingshot tried ten ports and all were taken.
* **Fix:** Choose another with `slingshot start --port <port>`.

### Cannot Keep The Agent Awake

* **Message:** `Could not stop this machine from sleeping`
* **Meaning:** A warning. If the Agent sleeps, nothing can reach it until it wakes. On Linux, a service started at boot with nobody logged in is not allowed to block sleep. GNOME's login screen suspends an idle machine after about 15 minutes. Issue #68 tracks a fix.
* **Fix:** Turn sleep off on the Agent with `sudo systemctl mask sleep.target suspend.target hibernate.target hybrid-sleep.target`. Slingshot then prints `Sleep is turned off on this machine` instead of this warning.

### No Relay Answered

* **Message:** `No iroh relay answered`
* **Meaning:** A warning. Machines on other networks cannot reach the Agent yet. The same network still works, and Slingshot keeps trying.
* **Fix:** Check the Agent's internet connection.

### Unreachable After A Restart

* **Meaning:** Slingshot does not start by itself on this Agent.
* **Fix:** Run `slingshot start --boot` on the Agent.

## Starting By Itself

### Set Up But Not Running

* **Message:** `Slingshot is set to start by itself but is not running`
* **Meaning:** The service file exists, but the service stopped or failed.
* **Fix:** Read its logs, shown in [Start By Itself](USAGE.md#start-by-itself). Then run `slingshot start --remove` and `slingshot start --boot`.

### Did Not Start

* **Message:** `Slingshot was set up to start at boot but did not start`
* **Meaning:** The service was installed, but did not answer within 15 seconds.
* **Fix:** Read its logs, fix what they report, then run `slingshot start --boot`.

### A Setup Command Failed

* **Message:** `systemctl --user enable --now slingshot.service failed: <reason>`, or the same for `launchctl`
* **Meaning:** The service manager refused a step. The reason comes from it.
* **Fix:** Fix the reason it gives, then run `slingshot start --boot`.

### Starts At Login, Not At Boot

* **Message:** `! Slingshot starts when you log in, not at boot`
* **Meaning:** Linux did not allow linger without an admin password, so the service waits for a login.
* **Fix:** Run the `sudo loginctl enable-linger <user>` command it prints.

### Already Running When Setting Up

* **Message:** `Slingshot is already running for this account. Stop the other slingshot start, then run slingshot start --boot again`
* **Meaning:** A `slingshot start` in another terminal holds the Agent, so the service could not start.
* **Fix:** Press Ctrl C in that terminal, then run `slingshot start --boot`.

### Remove Needs A Terminal

* **Message:** `Run slingshot start --remove in a terminal to confirm`
* **Meaning:** Removing asks first, and there was no terminal to ask in.
* **Fix:** Run it in a terminal on the Agent.

### Not Supported Here

* **Message:** `Starting at boot works on Linux and macOS`, or `A line break cannot go in a systemd unit`
* **Meaning:** The system has no supported service manager, or the Agent name or program path contains a line break.
* **Fix:** Run `slingshot start` by hand, or rename the Agent with `--name`.

## Linking

### Cannot Reach The Agent To Pair

* **Message:** `Could not reach 192.168.1.9:7433`, or `Could not reach the Agent at 192.168.1.9:7433`
* **Meaning:** The Client could not contact the Agent.
* **Fix:**

1. Check that `slingshot start` is running on the Agent.
2. Check that both machines are on the same network or tailnet. Linking across networks is not supported yet.

### Code Expired Or Used

* **Message:** `That pairing code has expired or was already used`
* **Meaning:** Each code works once, lasts 10 minutes, and is burned after 3 wrong tries.
* **Fix:** Press Enter where `slingshot start` is running for a new code.

### Wrong Code

* **Message:** `That pairing code is not right`
* **Meaning:** The code does not match the one the Agent printed. After 3 wrong tries the code stops working.
* **Fix:** Copy the code again, exactly as printed.

### Wrong Codes On The Agent

* **Message:** `Wrong pairing code from 192.168.1.40 (1 of 3)`, or `3 wrong pairing codes from 192.168.1.40. The code no longer works`
* **Meaning:** A machine at that address tried a wrong code. If it was not you, someone on your network tried to pair.
* **Fix:** Press Enter for a new code. Pair only with machines you recognize.

### Could Not Confirm The Agent

* **Message:** `Could not confirm that 192.168.1.9 is the Agent that printed this code`
* **Meaning:** The machine that answered could not prove it knows the code. Usually the code was mistyped. Rarely, another machine on the network is answering in the Agent's place.
* **Fix:** Check the code and try again. If it keeps failing on a network you trust, open an issue.

### Different Slingshot Versions

* **Message:** `The Agent at 192.168.1.9 runs an older Slingshot`, or `This Agent needs a newer Slingshot`
* **Meaning:** Pairing changed, so both machines need the same version to link. Machines that are already linked keep working.
* **Fix:** Update Slingshot on the machine the message names, restart `slingshot start` if it was the Agent, then link again.

### Another Machine Is Pairing

* **Message:** `Another machine is pairing right now`
* **Meaning:** The Agent pairs with one machine at a time.
* **Fix:** Wait a few seconds, then run `slingshot link` again.

### Not A Pairing Code

* **Message:** `That does not look like a pairing code`
* **Meaning:** A code has three parts separated by colons, such as `192.168.1.9:7433:K7QW9ZR2`.
* **Fix:** Copy the whole code from the Agent's screen.

## Setting Up Tools

### Tools Step Failed After Linking

* **Message:** `Could not set up tools on archbox`
* **Meaning:** Linking worked. Only the tools step at the end failed, and the reason follows the message.
* **Fix:** Fix the reason shown, then run `slingshot tools`.

### No Install Command

* **Message:** `No install command for brew. Install it on the Agent yourself`, or `No package manager found`
* **Meaning:** Slingshot only runs install commands it trusts on that kind of Agent.
* **Fix:** Install the tool on the Agent with its own instructions, for example inside `slingshot attach`.

### Tool Still Missing

* **Message:** `Docker is still missing. Check the output above, then run slingshot tools again`
* **Meaning:** An install command failed, or the tool was installed in a folder Slingshot does not search. Slingshot searches the Agent's login PATH plus `~/.local/bin` and `~/.cargo/bin`.
* **Fix:**

1. Read the error printed during the install, and fix it.
2. If the installer put the tool in another folder, add that folder to the PATH in the Agent's `~/.profile` or `~/.bash_profile`.
3. Run `slingshot tools` again.

### Sign In Did Not Finish

* **Message:** `Sign in to Codex did not finish`
* **Meaning:** The sign in was cancelled or failed. The tool is still installed.
* **Fix:**

1. Run the command shown in the message inside `slingshot attach`.
2. For Codex, nothing else on the Client may use port 1455 during the sign in.

### Docker Permission Denied

* **Message:** `permission denied while trying to connect to the Docker daemon socket`
* **Meaning:** This account is not in the `docker` group, or was added after sessions and Slingshot's service started. They keep the groups they started with.
* **Fix:** Add the account with `sudo usermod -aG docker <user>` if `slingshot start` warned about it, then restart the machine. `slingshot run` sees the change at once, but sessions only after the restart.

## Reaching The Agent

* When the Agent does not answer, Slingshot tries the local network, then the tailnet, then iroh.
* The message describes why the last attempt failed.

### Off Or Asleep

* **Message:** `archbox is not reachable. It may be off or asleep, or slingshot start is not running there`
* **Meaning:** Nothing answered on any path.
* **Fix:** Make sure the Agent is on and awake, and `slingshot start` is running.

### Slingshot Stopped On The Agent

* **Message:** `Slingshot is not running on the Agent. Run slingshot start there`
* **Meaning:** The Agent is on, but `slingshot start` has stopped.
* **Fix:** Run `slingshot start` on the Agent.

### No Longer Paired

* **Message:** `archbox no longer accepts this machine`
* **Meaning:** The Agent no longer recognizes this Client, usually after an unlink or a reset.
* **Fix:** Link again from the same network as the Agent, with `slingshot link <new code>`.

### No Internet On This Machine

* **Message:** `Could not reach archbox, because this machine cannot reach iroh's relays`
* **Meaning:** The Client has no working internet connection.
* **Fix:** Check this machine's connection, then try again.

### Versions Differ

* **Message:** `archbox runs an older Slingshot 0.1.0 than this machine (0.2.0)`, or `This machine runs an older Slingshot (0.1.0) than archbox 0.2.0`
* **Meaning:** The two machines run versions that cannot talk to each other. The message names the older one.
* **Fix:** On the machine it names, run `cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli`. If that is the Agent, restart `slingshot start` there.

### Different Releases

* **Message:** `! archbox runs Slingshot 0.1.0 and this machine runs 0.2.0`
* **Meaning:** Both machines still work together, but the older one is missing fixes.
* **Fix:** Run `cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli` on the older machine. If that is the Agent, restart `slingshot start` there.

### Slingshot Missing On The Agent

* **Message:** `Slingshot was not found on the Agent`
* **Meaning:** The `slingshot` program on the Agent was moved or removed.
* **Fix:** Install Slingshot on the Agent again, then run `slingshot link` again.

### Agent Identity Changed

* **Message:** `Host key verification failed`
* **Meaning:** The Agent's ssh identity changed, usually after a system reinstall.
* **Fix:** Link again so the Client learns the new identity.

### No Agent Linked

* **Message:** `No Agent configured yet`
* **Meaning:** This machine has not been linked.
* **Fix:** Run `slingshot start` on the Agent, then `slingshot link <code>` here.

### Several Agents Linked

* **Message:** `Several Agents configured`
* **Meaning:** More than one Agent is linked, and none is the default.
* **Fix:** Add `--agent <name>`, or set `default = "<name>"` in the config file the message names.

## Running Commands

### No Project Found

* **Message:** `No project found`
* **Meaning:** `sync` and `env` need a project, and this folder is not in one. `attach` works anywhere, and opens the home session outside a project.
* **Fix:** Move into a Git repository, or a folder with `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements.txt`, or `slingshot.toml`.

### Run Lost Its Connection

* **Message:** `Lost connection to archbox`, followed by `The Agent stops the run once it notices`
* **Meaning:** A run stops when its connection drops.
* **Fix:** Check how it ended with `slingshot ps --all`, where it shows as `Lost connection`. Use `slingshot attach` for work that must keep going.

### Command Not Found On The Agent

* **Message:** `cargo was not found on the Agent. Check the name, or install it there`, with exit 127
* **Meaning:** `slingshot run` looked for the program on the Agent and it is not there, or the name has a typo.
* **Fix:** Check the name. For a tool that works on the Client, run `slingshot tools` to install it on the Agent.

### Command Not Allowed To Run

* **Message:** `./build.sh is not allowed to run on the Agent. Check that it is executable`, with exit 126
* **Meaning:** The file exists but cannot be run, usually because it is not marked executable.
* **Fix:** Run `chmod +x build.sh` on the Client, then `slingshot run ./build.sh` again. The sync copies the change.

### Command Not Found In A Session

* **Message:** `command not found`, for a tool that works on the Client
* **Meaning:** A session runs on the Agent, so it can only use tools installed there.
* **Fix:** Run `slingshot tools` to install it on the Agent and sign in.

### Session Lost Its Connection

* **Message:** `Lost connection to archbox. The session keeps running there`
* **Meaning:** The session is still running on the Agent.
* **Fix:** Run `slingshot attach` to return to it.

### Edits Not Pulled After Leaving

* **Message:** `! Did not bring back edits: NOTES.md changed on both this machine and archbox`, where NOTES.md is the file and archbox is your Agent
* **Meaning:** You left the session, and the same files changed in both places. Nothing was overwritten. Both versions are kept.
* **Fix:** Follow [Conflict](#conflict), then run `slingshot sync --pull`.

### Low Memory Or Disk

* **Message:** `RAM is 92% used` or `Only 1.2 GiB free on the Agent workspace disk`
* **Meaning:** A warning before a run. The run still starts, but may be slow, fail, or be stopped.
* **Fix:** Free memory or disk space on the Agent.

### No Swap

* **Message:** `No swap configured`
* **Meaning:** The Agent has no swap, the disk space a system uses when RAM runs out. A large job may be stopped when memory fills.
* **Fix:** Add swap space on the Agent.

### Needs A Terminal

* **Message:** `Live views need an interactive terminal`
* **Meaning:** `slingshot health --watch` redraws the screen, which needs a real terminal.
* **Fix:** Run it in a terminal, or use `slingshot health` to save or pipe the output.

## Syncing Files

### Conflict

* **Message:** `NOTES.md changed on both this machine and archbox`, or `3 files changed on both this machine and archbox` followed by their names
* **Meaning:** The same files changed on both machines in different ways. Nothing was copied.
* **Fix:**

1. Compare both sides with `slingshot sync --check` and `slingshot sync --pull --check`.
2. Make each listed file match on both machines, or undo one edit.
3. Run the sync again.

### Names Differ Only In Case

* **Message:** `NOTES.md and notes.md differ only in case`
* **Meaning:** The Agent has two files whose names differ only in capital letters. This machine's disk treats them as one name, as a Mac does by default, so bringing both back would overwrite one. Nothing was changed.
* **Fix:** Rename or remove one of them on the Agent, then run the sync again.

### Interrupted Sync

* **Message:** `An interrupted sync of app needs recovery. Run slingshot sync to recover it`, where app is your project
* **Meaning:** A sync stopped partway, for example when the connection dropped.
* **Fix:** Run `slingshot sync`. It finishes or safely undoes the earlier sync.

### Recovery Failed

* **Message:** `Could not recover an interrupted sync. Backups are in ...`
* **Meaning:** Slingshot could not finish or undo the sync, and kept backups in the folder named.
* **Fix:** Make each listed file match its backup, or delete it, then sync again.

### No Copy Yet

* **Message:** `This project has no copy on the Agent yet. Run slingshot sync first`
* **Meaning:** A pull needs a copy on the Agent.
* **Fix:** Run `slingshot sync` once.

### Unsafe Path Or Link

* **Message:** `Unsupported source path` or `Unsafe source link`
* **Meaning:** A file name or link cannot be copied safely, such as a link pointing outside the project.
* **Fix:** Remove it, or add it to `.gitignore`.

### Too Many Files

* **Message:** `Message is too large. The project may have too many files`
* **Meaning:** The project's file list is over the limit.
* **Fix:** Leave out large generated or data folders with `.gitignore` or `sync.exclude`.

### Negated Exclude Pattern

* **Message:** `sync.exclude in slingshot.toml cannot contain negated patterns`
* **Meaning:** Patterns beginning with `!` are not allowed in `sync.exclude`.
* **Fix:** Remove them.

## Jobs

### No Matching Job

* **Message:** `No running Slingshot job matches abc123`
* **Meaning:** No running job has that ID. Finished jobs cannot be stopped.
* **Fix:** Copy the ID from `slingshot ps`.

## GPU

### GPU Reporting Unavailable

* **Message:** `GPU reporting unavailable, because nvidia-smi is not installed`
* **Meaning:** A warning from `slingshot start`. Slingshot reads GPU use with `nvidia-smi`, which comes with the NVIDIA driver.
* **Fix:** Nothing, if the Agent has no NVIDIA GPU. If it has one, install the NVIDIA driver, then run `slingshot start` again.

### Driver Mismatch

* **Message:** `NVIDIA driver and library versions differ`
* **Meaning:** The NVIDIA driver was updated, but the old one is still loaded.
* **Fix:** Restart the Agent.

### No GPU Data

* **Message:** `No GPU data available`
* **Meaning:** The Agent has no NVIDIA card, or `nvidia-smi` is not installed.
* **Fix:** None needed. Slingshot works without a GPU.

## The Menu Bar App

### Swift Not Found

* **Message:** `Swift is needed to build the menu bar app`
* **Meaning:** `slingshot menubar` builds the app with Swift, which comes with the Xcode command line tools.
* **Fix:** Run `xcode-select --install`, then `slingshot menubar` again.

### App Build Failed

* **Message:** `Could not build the menu bar app. Swift said:`, followed by Swift's last lines
* **Meaning:** Swift could not build the app. The app needs macOS 14 or later.
* **Fix:** Install updates for the Xcode command line tools in System Settings, then General, then Software Update. Run `slingshot menubar` again. If it still fails, open an issue with the full message.

### App Too Old

* **Message:** The panel says `Slingshot speaks format <n>, and this app speaks format <m>`
* **Meaning:** The app is older than the `slingshot` program it runs.
* **Fix:** Run `slingshot menubar`. It rebuilds the app.

### App Cannot Find Slingshot

* **Message:** The panel says `Can't find the slingshot program`, and names where it was.
* **Meaning:** Apps started at login cannot find the `slingshot` program on their own, so `slingshot menubar` saves its path. That program was moved or deleted, or was never saved.
* **Fix:** Run `slingshot menubar`. Run it again after moving `slingshot` to another folder.

### App Runs A Cargo Build

* **Message:** `! The app will run <path>, which cargo clean deletes`
* **Meaning:** `slingshot menubar` ran from a build inside a cargo `target` folder. When that build is deleted, the app cannot find Slingshot.
* **Fix:** Run `cargo install --path crates/slingshot-cli`, then `slingshot menubar` again.

### Could Not Leave Login Items

* **Message:** `Could not take the app out of login items`
* **Meaning:** `slingshot menubar --remove` starts the app once so it can leave login items, and that failed.
* **Fix:** Turn off Slingshot in System Settings, then General, then Login Items, then run `slingshot menubar --remove` again.

### Shown As Offline

* **Meaning:** The app cannot reach the Agent.
* **Fix:** Open the panel. It shows the cause and the fix. It updates by itself when the Agent answers, or press Try again.

### No Notifications

* **Meaning:** Notifications are off, or nothing has happened that needs one. Runs under 10 seconds never notify, and each warning is sent once until it clears.
* **Fix:** Turn them on in System Settings, then Notifications, then Slingshot.

## Removing Slingshot

### Key Still Installed

* **Message:** `Could not reach archbox, so the key is still there`
* **Meaning:** `slingshot unlink` could not reach the Agent to remove this machine's key.
* **Fix:** On the Agent, open `~/.ssh/authorized_keys` and delete the line ending with the name in the message.

### Job Still Running

* **Meaning:** `slingshot unlink` will not run while a job is active.
* **Fix:** Stop each job with `slingshot stop <id>`, then unlink.

## Getting Help

* Open an issue on GitHub with:

1. The command you ran.
2. What you expected.
3. The full output.
4. The operating system of each machine.

* [CONTRIBUTING.md](../CONTRIBUTING.md#reporting-a-problem) has the full list.
