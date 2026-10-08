//! The Agent daemon: pairing over TCP, plus the private control service used for
//! everything after pairing. Work itself runs through SSH.

pub mod awake;
pub mod boot;
pub mod clients;
pub mod jobs;
pub mod projects;
pub mod runner;
pub mod service;
pub mod tunnel;
pub mod uninstall;

use anyhow::Context;
use slingshot_core::keys;
use slingshot_core::network::Network;
use slingshot_core::preflight;
use slingshot_core::presentation::{self, Style, Tone};
use slingshot_core::protocol::{Handshake, Joining, Paired, Request, Response, Side};
use slingshot_core::step;
use slingshot_core::telemetry;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

/// How long a pairing code stays good for. Long enough to walk to the other machine,
/// short enough that a code left on screen overnight is worthless.
const CODE_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// Characters a pairing code is built from. No I, O, 0 or 1, because somebody is
/// going to read this off one screen and type it into another.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

const CODE_LENGTH: usize = 8;

/// Pairing requests are small. Anything larger, or slower than this, is dropped.
const PAIRING_LIMIT: u64 = 64 * 1024;
const PAIRING_TIMEOUT: Duration = Duration::from_secs(10);

/// Wrong codes a pairing code survives. Each one is a single online guess, so three
/// leave a guesser almost no chance, while a mistyped code can still be fixed.
const WRONG_CODE_LIMIT: u32 = 3;

/// What an older Client sees, since it cannot take part in the handshake.
const UPDATE_CLIENT: &str = "This Agent needs a newer Slingshot. Update Slingshot on this machine, then run slingshot link again";

/// The one time token that lets a Client install its key. Single use: pairing
/// consumes it, and there is no way to ask the daemon what it was.
struct Pairing {
    token: String,
    expires: Instant,
    wrong: u32,
}

impl Pairing {
    fn new() -> Pairing {
        Pairing {
            token: new_token(),
            expires: Instant::now() + CODE_LIFETIME,
            wrong: 0,
        }
    }
}

/// Everything the daemon needs while it runs. The name is chosen here and travels
/// to the Client at pairing, so the "running on" line never has to guess.
struct Agent {
    name: String,
    user: String,
    addresses: Vec<String>,
    root: PathBuf,
    iroh: String,
    pairing: Mutex<Option<Pairing>>,
    /// True while a handshake is under way. One at a time, so guesses cannot run in parallel.
    exchanging: AtomicBool,
}

/// What Ctrl C does where `slingshot start` runs: stop the daemon this process runs, or
/// only leave, when the daemon belongs to another `slingshot start` on this account.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Daemon,
    Beside,
}

impl Mode {
    fn ctrl_c(self) -> &'static str {
        match self {
            Mode::Daemon => "Ctrl C to stop",
            Mode::Beside => "Ctrl C to leave while Slingshot keeps running",
        }
    }
}

const REACHABLE: &str = "Reachable from other networks through iroh";

/// What `slingshot start` does about starting at boot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AtBoot {
    /// Ask once, the first time `slingshot start` runs in a terminal.
    Ask,
    /// `--boot`: set it up without asking, even after an earlier no.
    Install,
    /// `--remove`: stop the service and delete it.
    Remove,
}

/// How long a newly installed service has to answer on its socket, and to reach an iroh
/// relay, so the start screen does not warn about a relay it is still reaching.
const SERVICE_START: Duration = Duration::from_secs(15);

/// Start the daemon: run the checks, show the linked Clients or a pairing code, then listen.
/// When the daemon already runs for this account, show its status and pair beside it. The
/// first start in a terminal offers to run the daemon as a boot service instead.
pub async fn start(name: Option<String>, port: u16, at_boot: AtBoot) -> anyhow::Result<i32> {
    if at_boot == AtBoot::Remove {
        return boot::remove().await;
    }
    let mut checks = preflight::start_checks();
    checks.extend(
        whoami()
            .ok()
            .and_then(|user| preflight::docker_check(&user)),
    );
    if preflight::report(&checks) {
        anyhow::bail!("Fix the reported errors, then run slingshot start again");
    }

    let root = service::root()?;
    let installed = boot::installed()?;
    if service::running(&root)? {
        if at_boot == AtBoot::Install && !installed {
            anyhow::bail!(
                "Slingshot is already running for this account. Stop the other slingshot start, then run slingshot start --boot again"
            );
        }
        return beside(name, port, root).await;
    }

    let name = agent_name(name);
    if installed {
        presentation::warning(format!(
            "Slingshot is set to start by itself but is not running. Its file is {}. Run slingshot start --remove, then slingshot start --boot",
            presentation::home_path(&boot::file()?)
        ));
    } else if wants_boot(&root, &name, at_boot).await? {
        return beside(Some(name), port, root).await;
    }
    let online = Arc::new(AtomicBool::new(false));
    let _service = service::start(name.clone(), Arc::clone(&online))?;
    let identity = slingshot_core::tunnel::identity(&root)?;
    let linked = clients::names(&root)?;
    let (agent, addresses) = pairing(
        name.clone(),
        port,
        root,
        identity.public().to_string(),
        first_code(&linked),
    )
    .await?;

    let endpoint = tunnel::start(identity, agent.root.clone()).await?;
    tokio::spawn(mark_online(endpoint.clone(), online));

    let _awake = keep_awake();

    report_reach(&endpoint).await;
    show(
        &agent,
        &addresses,
        env!("CARGO_PKG_VERSION"),
        &linked,
        Mode::Daemon,
    );
    tokio::spawn(new_codes_on_enter(
        Arc::clone(&agent),
        addresses,
        Mode::Daemon,
    ));

    let stop = stopped().await?;
    endpoint.close().await;
    if stop != Stop::TerminalClosed {
        eprintln!();
        presentation::success(format!("Stopped Slingshot on {name}"));
    }

    Ok(0)
}

/// What a boot service runs: the control socket, iroh, and the lock that keeps the machine
/// awake. It never opens the pairing port, so no code exists until someone runs
/// `slingshot start`, which pairs beside it. The checks are left to that command, because
/// nobody is here to read them, and at boot sshd may not be up yet.
pub async fn daemon(name: Option<String>) -> anyhow::Result<i32> {
    let root = service::root()?;
    let name = agent_name(name);
    let online = Arc::new(AtomicBool::new(false));
    let _service = service::start(name.clone(), Arc::clone(&online))?;
    let identity = slingshot_core::tunnel::identity(&root)?;
    let endpoint = tunnel::start(identity, root).await?;
    tokio::spawn(mark_online(endpoint.clone(), online));
    let _awake = keep_awake();
    presentation::success(format!("Slingshot is running on {name}"));

    stopped().await?;
    endpoint.close().await;
    Ok(0)
}

/// Offer the boot service, install it on a yes, and wait until it answers. A no is saved, so
/// later starts go straight to the start screen.
async fn wants_boot(root: &std::path::Path, name: &str, at_boot: AtBoot) -> anyhow::Result<bool> {
    let ask = at_boot == AtBoot::Ask;
    if ask && (!interactive() || boot::declined(root)) {
        return Ok(false);
    }
    let user = whoami().context("Could not work out which user is running Slingshot")?;
    let plan = boot::plan(root, name, &user)?;
    if ask {
        eprintln!(
            "\n  Slingshot can start {} by itself, so linked Clients reach this machine without anyone here:",
            plan.when
        );
        boot::show(&plan);
        if !presentation::confirm(format!("Start Slingshot {}?", plan.when)).await? {
            boot::decline(root)?;
            presentation::detail("Later", "slingshot start --boot");
            return Ok(false);
        }
    } else {
        boot::show(&plan);
    }
    boot::install(&plan)?;

    let step = step::start("Starting Slingshot");
    let started = Instant::now();
    let mut answered = false;
    loop {
        match service::status(root).await {
            Ok(status) if status.online => break,
            Ok(_) => answered = true,
            Err(_) => {}
        }
        if started.elapsed() > SERVICE_START {
            if answered {
                break;
            }
            step.clear();
            anyhow::bail!(
                "Slingshot was set up to start {} but did not start. Its file is {}",
                plan.when,
                presentation::home_path(&plan.file)
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    step.done("Slingshot started");
    Ok(true)
}

/// Whether someone is at a terminal to answer a question.
fn interactive() -> bool {
    use std::io::IsTerminal;

    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn agent_name(name: Option<String>) -> String {
    name.or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "agent".to_string())
}

/// Hold the awake lock for as long as the result lives, and say whether it worked. A machine
/// that cannot sleep needs no lock.
fn keep_awake() -> Option<awake::Awake> {
    if awake::sleep_off() {
        presentation::success("Sleep is turned off on this machine, so it stays reachable");
        return None;
    }
    let held = awake::hold();
    match held {
        Some(_) => presentation::success("Keeping this machine awake while Slingshot runs"),
        None => presentation::warning(sleep_warning(std::env::var_os("SSH_CONNECTION").is_some())),
    }
    held
}

/// Pair beside a daemon another `slingshot start` runs, such as one started at boot. Pairing
/// only writes files the daemon reads fresh, so the code never has to reach the daemon.
async fn beside(name: Option<String>, port: u16, root: PathBuf) -> anyhow::Result<i32> {
    let status = service::status(&root).await?;
    if let Some(asked) = name.filter(|asked| *asked != status.name) {
        anyhow::bail!(
            "Slingshot is already running here as {}, not {asked}. Run slingshot start without --name",
            status.name
        );
    }
    let identity = slingshot_core::tunnel::identity(&root)?;
    let linked = clients::names(&root)?;
    let (agent, addresses) = pairing(
        status.name.clone(),
        port,
        root,
        identity.public().to_string(),
        first_code(&linked),
    )
    .await?;

    match status.online {
        true => presentation::success(REACHABLE),
        false => presentation::warning(
            "No iroh relay has answered yet, so other networks cannot reach this machine. The same network still works",
        ),
    }
    show(&agent, &addresses, &status.release, &linked, Mode::Beside);
    tokio::spawn(new_codes_on_enter(
        Arc::clone(&agent),
        addresses,
        Mode::Beside,
    ));

    if stopped().await? != Stop::TerminalClosed {
        eprintln!();
        presentation::success(format!("Slingshot keeps running on {}", status.name));
    }
    Ok(0)
}

/// An Agent that already has linked Clients leads with them, and makes a code only when
/// asked. A first start leads with a code, since linking is the only thing to do.
fn first_code(linked: &[String]) -> Option<Pairing> {
    linked.is_empty().then(Pairing::new)
}

/// Open the pairing port, and take connections on it until this process ends.
async fn pairing(
    name: String,
    port: u16,
    root: PathBuf,
    iroh: String,
    code: Option<Pairing>,
) -> anyhow::Result<(Arc<Agent>, Vec<SocketAddr>)> {
    let user = whoami().context("Could not work out which user is running the daemon")?;
    let (chosen, listeners) = listen(port).await?;
    if chosen != port {
        presentation::warning(format!(
            "Port {port} is in use, perhaps by another slingshot start, so this one uses {chosen}"
        ));
    }
    let addresses: Vec<SocketAddr> = listeners
        .iter()
        .filter_map(|listener| listener.local_addr().ok())
        .collect();

    let agent = Arc::new(Agent {
        name,
        user,
        addresses: addresses
            .iter()
            .filter(|addr| !addr.ip().is_loopback())
            .map(|addr| addr.ip().to_string())
            .collect(),
        root,
        iroh,
        pairing: Mutex::new(code),
        exchanging: AtomicBool::new(false),
    });

    for listener in listeners {
        tokio::spawn(accept_loop(listener, Arc::clone(&agent)));
    }
    Ok((agent, addresses))
}

/// Record when an iroh relay first answers, for `Status`. The endpoint keeps trying after
/// `report_reach` gives up, so a late answer still counts.
async fn mark_online(endpoint: iroh::Endpoint, online: Arc<AtomicBool>) {
    endpoint.online().await;
    online.store(true, Ordering::SeqCst);
}

#[derive(Debug, PartialEq)]
enum Stop {
    Asked,
    TerminalClosed,
}

/// Wait for Ctrl C, a request to stop, or the terminal closing. Each ends with iroh closed
/// cleanly, so linked Clients drop their shared connection at once instead of waiting on
/// one that is gone. A closed terminal has nowhere left to print.
async fn stopped() -> anyhow::Result<Stop> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut hangup = signal(SignalKind::hangup())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(tokio::select! {
        _ = tokio::signal::ctrl_c() => Stop::Asked,
        _ = terminate.recv() => Stop::Asked,
        _ = hangup.recv() => Stop::TerminalClosed,
    })
}

/// Why the machine may sleep. Over ssh the system only allows the lock with a password,
/// which Slingshot never asks for, so the fix is to start it at the machine.
fn sleep_warning(over_ssh: bool) -> &'static str {
    match over_ssh {
        true => {
            "Started over ssh, so this machine may still sleep. Run slingshot start at the machine to keep it awake"
        }
        false => {
            "Could not stop this machine from sleeping. If it sleeps, other machines cannot reach it until it wakes"
        }
    }
}

/// Say once whether other networks can reach this Agent, before the pairing code so the code
/// stays the last thing on screen. The same network works either way.
async fn report_reach(endpoint: &iroh::Endpoint) {
    let step = step::start("Connecting to iroh relays");
    match tunnel::online(endpoint).await {
        true => step.done(REACHABLE),
        false => {
            step.clear();
            presentation::warning(
                "No iroh relay answered, so other networks cannot reach this machine yet. The same network still works, and Slingshot keeps trying",
            );
        }
    }
}

/// Replace the pairing code each time Enter is pressed, for linking another machine or after
/// a code was burned. Stops quietly when there is no terminal to read.
async fn new_codes_on_enter(agent: Arc<Agent>, addresses: Vec<SocketAddr>, mode: Mode) {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(_)) = lines.next_line().await {
        let pairing = Pairing::new();
        let token = pairing.token.clone();
        *agent.pairing.lock().expect("pairing lock was poisoned") = Some(pairing);
        announce(&addresses, &token, mode);
    }
}

/// Take connections forever, one task each, so a slow Client never blocks another.
async fn accept_loop(listener: TcpListener, agent: Arc<Agent>) {
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let agent = Arc::clone(&agent);
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, peer, agent).await {
                        warn!("request from {peer} failed: {e:#}");
                    }
                });
            }
            Err(e) => warn!("Could not accept a connection: {e}"),
        }
    }
}

/// One pairing: `Start` and `Join` on the same connection. Anything that goes wrong comes
/// back as an Error response rather than a dropped connection, so the Client can explain it.
async fn handle(stream: TcpStream, peer: SocketAddr, agent: Arc<Agent>) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader.take(PAIRING_LIMIT));

    let response = match receive(&mut reader).await? {
        Err(message) => error(message),
        Ok(Request::Start { spake }) => match open(&agent) {
            Err(message) => error(message),
            Ok((token, handshake, ours, _exchange)) => {
                send(&mut writer, &Response::Start { spake: ours }).await?;
                match receive(&mut reader).await? {
                    Err(message) => error(message),
                    Ok(joining) => join(&agent, &token, handshake, &spake, joining, peer),
                }
            }
        },
        Ok(Request::Info | Request::Health | Request::Pair {}) => error(UPDATE_CLIENT),
        Ok(Request::Join { .. }) => {
            error("Pairing must begin with a handshake. Run slingshot link again")
        }
    };

    send(&mut writer, &response).await
}

/// Read one request, or the sentence explaining why it could not be understood.
async fn receive(
    reader: &mut BufReader<tokio::io::Take<OwnedReadHalf>>,
) -> anyhow::Result<Result<Request, String>> {
    let mut line = String::new();
    tokio::time::timeout(PAIRING_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("Pairing request timed out")??;
    Ok(serde_json::from_str::<Request>(line.trim())
        .map_err(|e| format!("Could not understand that request: {e}")))
}

async fn send(writer: &mut OwnedWriteHalf, response: &Response) -> anyhow::Result<()> {
    let mut reply = serde_json::to_string(response)?;
    reply.push('\n');
    writer.write_all(reply.as_bytes()).await?;
    Ok(())
}

fn error(message: impl Into<String>) -> Response {
    Response::Error {
        message: message.into(),
    }
}

/// Clears `exchanging` when a handshake ends, however it ends.
struct Exchange(Arc<Agent>);

impl Drop for Exchange {
    fn drop(&mut self) {
        self.0.exchanging.store(false, Ordering::SeqCst);
    }
}

/// Begin a handshake with the current code, if there is one and no other handshake is
/// under way. Returns the code it used, so `join` can tell if the code changed meanwhile.
fn open(agent: &Arc<Agent>) -> Result<(String, Handshake, Vec<u8>, Exchange), String> {
    let token = {
        let mut slot = agent.pairing.lock().expect("pairing lock was poisoned");
        if slot.as_ref().is_some_and(|p| Instant::now() > p.expires) {
            *slot = None;
        }
        match slot.as_ref() {
            Some(pairing) => pairing.token.clone(),
            None => return Err(EXPIRED.to_string()),
        }
    };
    if agent.exchanging.swap(true, Ordering::SeqCst) {
        return Err("Another machine is pairing right now. Try again in a few seconds".to_string());
    }
    let exchange = Exchange(Arc::clone(agent));
    let (handshake, ours) = Handshake::agent(&token);
    Ok((token, handshake, ours, exchange))
}

const EXPIRED: &str = "That pairing code has expired or was already used. Press Enter where slingshot start is running for a new one";

/// Check the Client's proof, then use up the code and install the Client's keys. A wrong
/// proof counts against the code, and the last allowed one burns it.
fn join(
    agent: &Agent,
    token: &str,
    handshake: Handshake,
    theirs: &[u8],
    request: Request,
    peer: SocketAddr,
) -> Response {
    let Request::Join { details, proof } = request else {
        return error("Pairing stopped halfway. Run slingshot link again");
    };
    let key = match handshake.finish(theirs) {
        Ok(key) => key,
        Err(e) => return error(format!("{e:#}. Run slingshot link again")),
    };

    let right = key.check(Side::Client, &details, &proof);
    {
        let mut slot = agent.pairing.lock().expect("pairing lock was poisoned");
        let current = slot
            .as_mut()
            .filter(|p| p.token == token && Instant::now() <= p.expires);
        match (current, right) {
            (None, _) => return error(EXPIRED),
            (Some(pairing), false) => {
                pairing.wrong += 1;
                let wrong = pairing.wrong;
                if wrong >= WRONG_CODE_LIMIT {
                    *slot = None;
                    presentation::warning(format!(
                        "{wrong} wrong pairing codes from {}. The code no longer works. Press Enter for a new one",
                        peer.ip()
                    ));
                } else {
                    presentation::warning(format!(
                        "Wrong pairing code from {} ({wrong} of {WRONG_CODE_LIMIT})",
                        peer.ip()
                    ));
                }
                return error("That pairing code is not right");
            }
            (Some(_), true) => *slot = None,
        }
    }

    let joining: Joining = match serde_json::from_str(&details) {
        Ok(joining) => joining,
        Err(e) => return error(format!("Could not understand the Client's details: {e}")),
    };
    let paired = match accept(agent, &joining) {
        Ok(paired) => paired,
        Err(e) => return error(format!("Could not finish pairing: {e:#}")),
    };
    match serde_json::to_string(&paired) {
        Ok(details) => Response::Paired {
            proof: key.prove(Side::Agent, &details),
            details,
        },
        Err(e) => error(format!("Could not send the pairing details: {e}")),
    }
}

/// Authorize the Client key for SSH execution and private control, and its iroh key for
/// reaching sshd from other networks. An older Client without an iroh key pairs as before.
fn accept(agent: &Agent, client: &Joining) -> anyhow::Result<Paired> {
    if let Some(iroh) = &client.iroh {
        clients::allow(&agent.root, &client.client, iroh)
            .context("Could not record the Client's iroh key")?;
    }
    let authorized = keys::authorize(&client.client, &client.public_key)
        .context("Could not add the Client's key to authorized_keys")?;

    info!("paired with {}", client.client);
    presentation::success(format!("Paired with {}", client.client));
    presentation::detail("Authorized", presentation::home_path(&authorized));

    Ok(Paired {
        name: agent.name.clone(),
        user: agent.user.clone(),
        host_keys: keys::host_keys(),
        program: std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_string)),
        addresses: agent.addresses.clone(),
        iroh: Some(agent.iroh.clone()),
        specs: telemetry::specs(&agent.name),
    })
}

/// Generate a token using operating system randomness.
fn new_token() -> String {
    (0..CODE_LENGTH)
        .map(|_| CODE_ALPHABET[rand::random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

/// The account the daemon is running as, which is who the installed key will log in as.
fn whoami() -> anyhow::Result<String> {
    let out = std::process::Command::new("id").arg("-un").output()?;

    if !out.status.success() {
        anyhow::bail!("id -un failed");
    }

    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// How many ports after the requested one to try, so several accounts on one machine can
/// each run slingshot start.
const PORTS_TO_TRY: u16 = 10;

/// Listen on every pairing address at the first port where none is taken. An address that
/// cannot listen for another reason is left out, as long as this machine itself can.
async fn listen(port: u16) -> anyhow::Result<(u16, Vec<TcpListener>)> {
    for candidate in (port..=port.saturating_add(PORTS_TO_TRY - 1)).filter(|p| *p != 0) {
        let mut listeners = Vec::new();
        let mut taken = false;
        for addr in bind_addresses(candidate) {
            match TcpListener::bind(addr).await {
                Ok(listener) => listeners.push(listener),
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    taken = true;
                    break;
                }
                Err(e) => warn!("Could not listen on {addr}: {e}"),
            }
        }
        let on_this_machine = listeners
            .first()
            .and_then(|listener| listener.local_addr().ok())
            .is_some_and(|addr| addr.ip().is_loopback());
        if !taken && on_this_machine {
            return Ok((candidate, listeners));
        }
    }
    anyhow::bail!(
        "Ports {port} to {} are all in use. Choose another with: slingshot start --port <port>",
        port.saturating_add(PORTS_TO_TRY - 1)
    )
}

/// Loopback plus whichever local networks this machine is actually on. Never
/// 0.0.0.0, so the daemon is not offered to an interface nobody asked about.
fn bind_addresses(port: u16) -> Vec<SocketAddr> {
    let mut addresses = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))];

    for probe in ["8.8.8.8:53", "100.100.100.100:53"] {
        if let Some(ip) = local_ip_towards(probe) {
            let addr = SocketAddr::new(ip, port);
            if !addresses.contains(&addr) {
                addresses.push(addr);
            }
        }
    }

    addresses
}

/// Which of this machine's addresses would be used to reach somewhere else. UDP
/// connect only picks a route, so nothing is sent and the target is never contacted.
fn local_ip_towards(target: &str) -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(target).ok()?;

    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if ip.is_loopback() || ip.is_unspecified() => None,
        ip => Some(ip),
    }
}

/// The start screen: which Agent runs and which release, then the pairing code, or the
/// linked Clients and how to ask for a code.
fn show(agent: &Agent, addresses: &[SocketAddr], release: &str, linked: &[String], mode: Mode) {
    let style = Style::stderr();
    eprintln!();
    eprintln!(
        "{} Slingshot {} is running on {}",
        style.paint("▶", Tone::Info),
        style.dim(release),
        style.paint(&agent.name, Tone::Info)
    );
    let token = agent
        .pairing
        .lock()
        .expect("pairing lock was poisoned")
        .as_ref()
        .map(|pairing| pairing.token.clone());
    match token {
        Some(token) => announce(addresses, &token, mode),
        None => {
            eprintln!();
            eprint!("{}", presentation::row("Linked", linked.join(", ")));
            eprintln!();
            eprintln!("  {}", style.dim(ask_for_code(mode)));
            eprintln!();
        }
    }
}

fn code_footer(mode: Mode) -> String {
    format!(
        "The code works once and expires in {} minutes. Press Enter for a new code, or {}",
        CODE_LIFETIME.as_secs() / 60,
        mode.ctrl_c()
    )
}

fn ask_for_code(mode: Mode) -> String {
    format!(
        "Press Enter for a code to link another machine, or {}",
        mode.ctrl_c()
    )
}

/// Print the pairing token only to the owner's console, never to logs or files.
/// Show a complete copyable command for each available address.
fn announce(addresses: &[SocketAddr], token: &str, mode: Mode) {
    let reachable: Vec<&SocketAddr> = addresses.iter().filter(|a| !a.ip().is_loopback()).collect();

    let offered = match reachable.is_empty() {
        true => addresses.iter().collect(),
        false => reachable,
    };

    let style = Style::stderr();
    eprintln!();

    if offered.is_empty() {
        eprintln!("  No address to pair on");
        return;
    }

    eprintln!("  To pair, run this on a machine on the same network or tailnet:");
    eprintln!();

    let commands: Vec<(String, &str)> = offered
        .iter()
        .map(|addr| {
            (
                format!("slingshot link {}:{}:{token}", addr.ip(), addr.port()),
                Network::of(&addr.ip().to_string()).name(),
            )
        })
        .collect();

    let width = commands.iter().map(|(c, _)| c.len()).max().unwrap_or(0);

    for (command, label) in &commands {
        eprintln!(
            "    {}   {}",
            style.paint(format!("{command:<width$}"), Tone::Info),
            style.dim(label)
        );
    }

    eprintln!();
    eprintln!("  {}", style.dim(code_footer(mode)));
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_taken_port_moves_to_the_next_free_one() {
        let taken = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let (chosen, listeners) = listen(port).await.unwrap();

        assert_ne!(chosen, port);
        assert!(chosen > port && chosen < port + PORTS_TO_TRY);
        assert!(listeners[0].local_addr().unwrap().ip().is_loopback());
    }
    use crate::testing::Root;

    fn agent(root: &Root, pairing: Pairing) -> Arc<Agent> {
        Arc::new(Agent {
            name: "archbox".to_string(),
            user: "me".to_string(),
            addresses: Vec::new(),
            root: root.0.clone(),
            iroh: String::new(),
            pairing: Mutex::new(Some(pairing)),
            exchanging: AtomicBool::new(false),
        })
    }

    fn code(token: &str) -> Pairing {
        Pairing {
            token: token.to_string(),
            expires: Instant::now() + CODE_LIFETIME,
            wrong: 0,
        }
    }

    /// A full handshake where the Client typed `typed`, stopped before any keys are installed
    /// unless the code was right.
    fn attempt(agent: &Arc<Agent>, typed: &str) -> Response {
        let (token, handshake, to_client, _exchange) = open(agent).unwrap();
        let (client, to_agent) = Handshake::client(typed);
        let key = client.finish(&to_client).unwrap();
        let details = r#"{"client":"laptop","public_key":"","iroh":null}"#.to_string();
        let proof = key.prove(Side::Client, &details);
        let peer = SocketAddr::from(([192, 168, 1, 40], 50000));
        join(
            agent,
            &token,
            handshake,
            &to_agent,
            Request::Join { details, proof },
            peer,
        )
    }

    fn message(response: Response) -> String {
        match response {
            Response::Error { message } => message,
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn a_first_start_leads_with_a_code_and_a_linked_agent_does_not() {
        assert!(first_code(&[]).is_some());
        assert!(first_code(&["laptop".to_string()]).is_none());
    }

    #[test]
    fn a_chosen_name_wins_over_the_hostname() {
        assert_eq!(agent_name(Some("archbox".into())), "archbox");
        assert!(!agent_name(None).is_empty());
    }

    #[test]
    fn leaving_beside_the_daemon_says_it_keeps_running() {
        assert!(code_footer(Mode::Daemon).ends_with("Ctrl C to stop"));
        assert!(ask_for_code(Mode::Beside).ends_with("Slingshot keeps running"));
    }

    #[test]
    fn the_sleep_warning_says_what_to_do_over_ssh() {
        assert!(sleep_warning(true).contains("Run slingshot start at the machine"));
        assert!(sleep_warning(false).contains("Could not stop this machine from sleeping"));
    }

    #[test]
    fn three_wrong_codes_burn_the_code() {
        let root = Root::new();
        let agent = agent(&root, code("K7QW9ZR2"));

        for _ in 0..WRONG_CODE_LIMIT {
            assert!(message(attempt(&agent, "K7QW9ZR3")).contains("not right"));
        }

        assert!(agent.pairing.lock().unwrap().is_none());
        assert!(open(&agent).is_err());
    }

    #[test]
    fn a_wrong_code_below_the_limit_leaves_the_code_usable() {
        let root = Root::new();
        let agent = agent(&root, code("K7QW9ZR2"));

        attempt(&agent, "K7QW9ZR3");

        assert_eq!(agent.pairing.lock().unwrap().as_ref().unwrap().wrong, 1);
        assert!(open(&agent).is_ok());
    }

    #[test]
    fn only_one_handshake_runs_at_a_time() {
        let root = Root::new();
        let agent = agent(&root, code("K7QW9ZR2"));

        let first = open(&agent).unwrap();
        assert!(open(&agent).err().unwrap().contains("Another machine"));
        drop(first);
        assert!(open(&agent).is_ok());
    }

    #[test]
    fn an_expired_code_refuses_to_start() {
        let root = Root::new();
        let mut expired = code("K7QW9ZR2");
        expired.expires = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        let agent = agent(&root, expired);

        assert_eq!(open(&agent).err().unwrap(), EXPIRED);
    }

    #[test]
    fn a_code_replaced_during_the_handshake_is_refused() {
        let root = Root::new();
        let agent = agent(&root, code("K7QW9ZR2"));
        let (token, handshake, to_client, _exchange) = open(&agent).unwrap();
        *agent.pairing.lock().unwrap() = Some(code("NEWCODE2"));

        let (client, to_agent) = Handshake::client("K7QW9ZR2");
        let key = client.finish(&to_client).unwrap();
        let details = "{}".to_string();
        let proof = key.prove(Side::Client, &details);
        let peer = SocketAddr::from(([192, 168, 1, 40], 50000));
        let response = join(
            &agent,
            &token,
            handshake,
            &to_agent,
            Request::Join { details, proof },
            peer,
        );

        assert_eq!(message(response), EXPIRED);
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;

    /// A private temporary Agent root removed when the test ends.
    pub struct Root(pub PathBuf);

    impl Root {
        pub fn new() -> Root {
            let path = std::env::temp_dir().join(format!(
                "slingshot-agent-{}",
                slingshot_core::storage::new_id()
            ));
            slingshot_core::storage::private_dir(&path).unwrap();
            Root(path.canonicalize().unwrap())
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
