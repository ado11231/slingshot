# Troubleshooting

* What each Slingshot error message means, and how to fix it.
* Search this page for the words on your screen. Each entry shows the exact message.

## Contents

1. [Starting The Agent](#starting-the-agent)
2. [Linking](#linking)
3. [Setting Up Tools](#setting-up-tools)
4. [Reaching The Agent](#reaching-the-agent)
5. [Running Commands](#running-commands)
6. [Syncing Files](#syncing-files)
7. [Jobs](#jobs)
8. [GPU](#gpu)
9. [The Menu Bar App](#the-menu-bar-app)
10. [Removing Slingshot](#removing-slingshot)
11. [Getting Help](#getting-help)

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

### Already Running

* **Message:** `Another slingshot start is already running for this account`
* **Meaning:** Only one `slingshot start` can run per user account.
* **Fix:** Use the one already running, or stop it with Ctrl C and start again.

### Port In Use

* **Message:** `Could not listen on any address; is port 7433 already in use?`
* **Meaning:** Another program is using the pairing port.
* **Fix:** Choose another port with `slingshot start --port 7434`.

### Cannot Keep The Agent Awake

* **Message:** `Could not stop this machine from sleeping`
* **Meaning:** A warning. If the Agent sleeps, nothing can reach it until it wakes.
* **Fix:** Change the Agent's power settings so it stays awake.

### No Relay Answered

* **Message:** `No iroh relay answered`
* **Meaning:** A warning. Machines on other networks cannot reach the Agent yet. The same network still works, and Slingshot keeps trying.
* **Fix:** Check the Agent's internet connection.

### Unreachable After A Restart

* **Meaning:** `slingshot start` does not yet start on its own when the Agent turns on.
* **Fix:** Log in to the Agent and run `slingshot start`.

## Linking

### Cannot Reach The Agent To Pair

* **Message:** `Could not reach 192.168.1.9:7433`
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
* **Meaning:** You were added to the `docker` group, but open sessions started before that.
* **Fix:** End open sessions with `exit`, then run `slingshot attach` again.

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

* **Message:** `Slingshot versions differ between the machines`
* **Meaning:** The two machines run versions that cannot talk to each other.
* **Fix:** Update Slingshot on both machines. Link again if Slingshot asks you to.

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

* **Message:** `Lost connection to archbox. The Agent stops the run once it notices`
* **Meaning:** A run stops when its connection drops.
* **Fix:** Check how it ended with `slingshot ps --all`. Use `slingshot attach` for work that must keep going.

### Command Not Found In A Session

* **Message:** `command not found`, for a tool that works on the Client
* **Meaning:** A session runs on the Agent, so it can only use tools installed there.
* **Fix:** Run `slingshot tools` to install it on the Agent and sign in.

### Session Lost Its Connection

* **Message:** `Lost connection to archbox. The session keeps running there`
* **Meaning:** The session is still running on the Agent.
* **Fix:** Run `slingshot attach` to return to it.

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
* **Meaning:** `slingshot health --watch` and `slingshot top` redraw the screen, which needs a real terminal.
* **Fix:** Run them in a terminal, or use `slingshot health` to save or pipe the output.

## Syncing Files

### Conflict

* **Message:** `These paths changed differently on this machine and archbox`
* **Meaning:** The same files changed on both machines in different ways. Nothing was copied.
* **Fix:**

1. Compare both sides with `slingshot sync --check` and `slingshot sync --pull --check`.
2. Make each listed file match on both machines, or undo one edit.
3. Run the sync again.

### Interrupted Sync

* **Message:** `An interrupted sync needs recovery. Run slingshot sync to recover it`
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

* **Message:** The panel says `Update the app with: slingshot menubar`
* **Meaning:** The app is older than the `slingshot` program it runs.
* **Fix:** Run `slingshot menubar`. It rebuilds the app.

### App Cannot Find Slingshot

* **Message:** The panel asks you to run `slingshot menubar` once from a terminal.
* **Meaning:** Apps started at login cannot find the `slingshot` program on their own.
* **Fix:** Run `slingshot menubar`. Run it again after moving `slingshot` to another folder.

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
