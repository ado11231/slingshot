<p align="center"><img src="https://raw.githubusercontent.com/ado11231/slingshot/master/docs/media/slingshot.png" alt="Slingshot logo, a pixel art slingshot" width="96"></p>

<h3 align="center">slingshot</h3>

<p align="center">
  <a href="https://github.com/ado11231/slingshot/actions/workflows/ci.yml"><img src="https://github.com/ado11231/slingshot/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/slingshot-cli"><img src="https://img.shields.io/crates/v/slingshot-cli" alt="crates.io"></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey" alt="macOS and Linux">
</p>

<p align="center">use the CPU, RAM, and GPU of another computer from your own. Run terminal processes securely, with local models coming next.</p>

<p align="center"><img src="https://raw.githubusercontent.com/ado11231/slingshot/master/docs/media/demo.gif" alt="Starting Slingshot on the powerful machine, linking the laptop, and opening a session" width="830"></p>

## What It Does

* **Remote power.** Builds, servers, containers, and coding agents run on the powerful machine, started from your laptop.
* **Feels local.** Typing, colors, Ctrl C, and exit codes work as they do on your own machine.
* **Your tools.** You keep your own editor, terminal, and browser. Nothing new to learn.
* **Light sync.** Only source files are copied. Build output such as `target` and `node_modules` stays on the powerful machine.
* **Lasting sessions.** `slingshot attach` opens a terminal that keeps running when you close the laptop.
* **Works anywhere.** At home, over a VPN such as Tailscale, or from any network, with no account and no router setup.
* **Secure.** The pairing code never crosses the network, each laptop gets its own ssh key, and nothing runs as another user.

## Setup

* The **Client** is the machine you work on. The **Agent** is the powerful machine.

1. Install Slingshot on both machines. It needs Rust and `rsync`, and the Agent also needs `tmux` and an ssh server. See [A Linux Agent](#a-linux-agent) below, or the [install guide](docs/USAGE.md#install) for other systems.

   ```sh
   cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli
   ```

2. On the Agent, start Slingshot. The first time, it offers to start by itself at boot. Then it prints a pairing code.

   ```sh
   slingshot start
   ```

3. On the Client, run the line it printed. Both machines must be on the same network or VPN for this step only.

   ```sh
   slingshot link 192.168.1.9:7433:K7QW9ZR2
   ```

4. From a project folder on the Client, run anything on the Agent:

   ```sh
   slingshot run cargo build
   ```

   `sling` is the short name, so `sling run cargo build` works too.

### A Linux Agent

1. Install what Slingshot needs. On Arch Linux:

   ```sh
   sudo pacman -S --needed base-devel rsync tmux openssh rustup
   rustup default stable
   ```

   On Ubuntu or Debian:

   ```sh
   sudo apt install build-essential rsync tmux openssh-server curl
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Turn on the ssh server, which the Client uses to log in. It is `sshd` on Arch and Fedora, and `ssh` on Ubuntu and Debian.

   ```sh
   sudo systemctl enable --now sshd
   ```

3. Add Rust's folder to your PATH, so the `slingshot` command is found. Use `~/.zshrc` instead if your shell is zsh. Then open a new terminal.

   ```sh
   echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
   ```

4. ssh keys need no setup. `slingshot link` makes a key for the Client and installs it in the Agent's `~/.ssh/authorized_keys`, named so `slingshot unlink` can remove it. Password login is never used.

## Commands

| Command | Does |
| --- | --- |
| `slingshot run <command>` | Copies your changes, then runs the command on the Agent. |
| `slingshot attach` | Opens a terminal on the Agent that keeps running when you disconnect. |
| `slingshot sync --pull` | Brings edits made on the Agent back to the Client. |
| `slingshot health` | Shows the Agent's hardware and use. Add `--watch` to keep it live. |
| `slingshot ps` | Lists running jobs. `slingshot stop <id>` stops one. |

* Every command and option is in [USAGE.md](docs/USAGE.md).

## Menu Bar

* On a Mac, `link` offers a menu bar app that shows the Agent's live use and notifies you when a run finishes.

<p align="center"><img src="https://raw.githubusercontent.com/ado11231/slingshot/master/docs/media/menubar.png" alt="The Slingshot menu bar panel" width="360"></p>

## Status

* Early. Tested between a Mac and an Arch Linux machine.
* What works is in the [roadmap](docs/ROADMAP.md). Planned work is in the [issues](https://github.com/ado11231/slingshot/issues).
* Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your choice.

<p align="center">
  <a href="docs/USAGE.md">Usage</a> ·
  <a href="docs/TROUBLESHOOTING.md">Troubleshooting</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="CONTRIBUTING.md">Contributing</a>
</p>
