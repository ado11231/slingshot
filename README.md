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

* The **Agent** is the powerful machine. The **Client** is the one you work on, such as your laptop.
* Set up the Agent first, then the Client. Each takes a few minutes, mostly while Rust builds Slingshot.

### 1. The Agent

**1. Install what Slingshot needs.** These commands need an admin account (`sudo`). On Arch Linux:

```sh
sudo pacman -S --needed base-devel rsync tmux openssh rustup
sudo systemctl enable --now sshd
rustup default stable
```

* Ubuntu, Debian, Fedora, and a Mac as the Agent are in the [install guide](docs/USAGE.md#install).
* Without `sudo`, ask an admin to run the first two lines. Check that everything is there with `command -v rsync tmux cargo`.

**2. Install Slingshot.**

```sh
cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli
```

**3. Check that it is found.**

```sh
slingshot --version
```

If it says `command not found`, add Rust's folder to your PATH, then open a new terminal. Use `~/.zshrc` instead if your shell is zsh.

```sh
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
```

**4. Start Slingshot.**

```sh
slingshot start
```

It checks the machine, offers to start by itself at boot, then prints a line such as `slingshot link 192.168.1.9:7433:K7QW9ZR2`. Keep it for the Client.

### 2. The Client

**1. Install Rust.** On a Mac:

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

A Linux Client needs `rsync` and Rust, but not `tmux` or an ssh server. See the [install guide](docs/USAGE.md#install).

**2. Install Slingshot and check it**, as on the Agent.

```sh
cargo install --locked --git https://github.com/ado11231/slingshot slingshot-cli
slingshot --version
```

**3. Run the line the Agent printed.** Both machines must be on the same network or VPN for this step only.

```sh
slingshot link 192.168.1.9:7433:K7QW9ZR2
```

It pairs, offers to install the tools you use on the Agent, such as Claude Code, offers to sign in to them, and on a Mac adds the menu bar app. Each asks first.

**4. Run anything on the Agent** from a project folder.

```sh
slingshot run cargo build
```

`sling` is the short name, so `sling run cargo build` works too.

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
