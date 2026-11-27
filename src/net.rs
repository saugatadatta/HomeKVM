//! Finding, pairing and staying connected with other HomeKVM PCs.
//!
//! - Every PC listens for encrypted QUIC connections (UDP, port 47820 if free).
//! - Every PC announces itself on the local network with mDNS and watches for others.
//! - Pairing: one PC asks, both show the same 6-digit code (derived from the TLS
//!   session), and the user confirms on both. Each PC then remembers the other's ID.
//! - Paired PCs reconnect on their own: the PC with the smaller ID dials.
//! - Keyboard and mouse sharing: when the cursor reaches an edge with a paired PC
//!   placed there, this PC becomes the controller and sends its input to that PC
//!   (see input.rs). The target reports when the cursor is pushed past its own edges,
//!   so the mouse can move along the row of PCs or come home.
//! - Shared clipboard: a copy made on this PC goes to every connected paired PC
//!   (see clipboard.rs); copies from them replace this PC's clipboard.
//!
//! Runs on its own thread with a small single-threaded async runtime.

use crate::config::Layout;
use crate::identity::{self, Identity};
use crate::clipboard::{self, Clip};
use crate::input::{self, Local};
use crate::peers::{self, Paired};
use crate::proto::{self, Edge, Msg};
use crate::state::{NetView, PairingView, PeerView, Shared};
use crate::{tls, win};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Progress lines in the console, development builds only.
macro_rules! dlog {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!("[net] {}", format!($($arg)*));
        }
    };
}

const SERVICE: &str = "_homekvm._udp.local.";
const PORT: u16 = 47820;
const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);
/// Unpaired connections are only for pairing; close them if nothing happens.
const UNPAIRED_IDLE: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Requests from the window.
pub enum Cmd {
    Pair(String),
    /// The user compared the codes: true = they match, false = cancel.
    Confirm(String, bool),
    Forget(String),
    Rename(String),
    ClearNotice,
}

#[derive(Clone)]
pub struct NetHandle {
    tx: UnboundedSender<Cmd>,
}

impl NetHandle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

/// Start the network thread. `on_change` runs whenever the shared NetView changes.
pub fn start(shared: Arc<Shared>, name: String, on_change: Arc<dyn Fn() + Send + Sync>) -> NetHandle {
    let (tx, rx) = unbounded_channel();
    // Keyboard and mouse events from this PC's hooks.
    let (local_tx, local_rx) = unbounded_channel();
    input::start(local_tx);
    // Copies made on this PC.
    let (clip_tx, clip_rx) = unbounded_channel();
    clipboard::start(clip_tx);
    std::thread::Builder::new()
        .name("net".into())
        .spawn(move || {
            // Screen coordinates in physical pixels for replaying input.
            unsafe {
                windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
                    windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                );
            }
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => return fail(&shared, &on_change, e.to_string()),
            };
            let result = runtime.block_on(run(shared.clone(), name, rx, local_rx, clip_rx, on_change.clone()));
            if let Err(e) = result {
                fail(&shared, &on_change, e);
            }
        })
        .expect("start network thread");
    NetHandle { tx }
}

fn fail(shared: &Shared, on_change: &Arc<dyn Fn() + Send + Sync>, error: String) {
    let mut view = shared.net();
    view.notice = Some(format!("Networking couldn't start: {error}"));
    shared.set_net(view);
    on_change();
}

struct Net {
    me: String,
    endpoint: quinn::Endpoint,
    shared: Arc<Shared>,
    on_change: Arc<dyn Fn() + Send + Sync>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    name: String,
    name_changed: bool,
    peers: BTreeMap<String, Peer>,
    pairing: Option<Pairing>,
    /// Waiting for a connection so pairing can start.
    pending_pair: Option<String>,
    connecting: HashSet<String>,
    notice: Option<String>,
    /// mDNS full name -> peer ID, to handle "service removed".
    mdns_names: HashMap<String, String>,
    /// This PC's keyboard and mouse control another PC right now.
    control: Option<Control>,
    /// Another PC controls this one right now.
    controlled_by: Option<String>,
}

/// Which PC gets this PC's input, and where it sits in the row (negative = left).
struct Control {
    target: String,
    pos: i32,
}

/// The paired PC at a row position: 1 is next to this PC on the right, -1 on the left.
fn peer_at(layout: &Layout, pos: i32) -> Option<String> {
    match pos {
        p if p > 0 => layout.right.get(p as usize - 1).cloned(),
        p if p < 0 => layout.left.get((-p) as usize - 1).cloned(),
        _ => None,
    }
}

#[derive(Default)]
struct Peer {
    name: String,
    paired: bool,
    found: bool,
    addrs: Vec<SocketAddr>,
    link: Option<Link>,
}

struct Link {
    conn: quinn::Connection,
    tx: UnboundedSender<Msg>,
    since: Instant,
}

struct Pairing {
    id: String,
    code: String,
    incoming: bool,
    local_ok: bool,
    remote_ok: bool,
    started: Instant,
}

impl Inner {
    fn online(&self, id: &str) -> bool {
        self.peers.get(id).is_some_and(|p| p.paired && p.link.is_some())
    }

    /// Input comes back to this PC; `place` puts the cursor at that edge.
    fn stop_control(&mut self, place: Option<(Edge, f32)>) {
        if let Some(c) = self.control.take() {
            self.send(&c.target, Msg::Leave);
            input::release(place);
        }
    }

    /// Another PC stops controlling this one.
    fn end_controlled(&mut self) {
        if self.controlled_by.take().is_some() {
            input::inject_release_all();
            input::set_controlled(false);
        }
    }

    fn send(&self, id: &str, msg: Msg) {
        if let Some(link) = self.peers.get(id).and_then(|p| p.link.as_ref()) {
            let _ = link.tx.send(msg);
        }
    }

    fn name_of(&self, id: &str) -> String {
        self.peers.get(id).map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "The other PC".into())
    }

    fn save_paired(&self) {
        let list: Vec<Paired> = self
            .peers
            .iter()
            .filter(|(_, p)| p.paired)
            .map(|(id, p)| Paired { id: id.clone(), name: p.name.clone() })
            .collect();
        if let Err(e) = peers::save(&list) {
            eprintln!("saving paired PCs: {e}");
        }
    }

    /// Both users confirmed: trust each other from now on.
    fn finish_pairing_if_done(&mut self) {
        let done = self.pairing.as_ref().is_some_and(|p| p.local_ok && p.remote_ok);
        if !done {
            return;
        }
        let id = self.pairing.take().map(|p| p.id).unwrap_or_default();
        dlog!("paired with {}", identity::short(&id));
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.paired = true;
        }
        self.notice = None;
        self.save_paired();
        // Tell the other side we trust it now (it reached the same result).
        let name = self.name.clone();
        self.send(&id, Msg::Hello { name, paired: true });
    }

    /// Forget a PC that's neither trusted, connected nor visible.
    fn prune(&mut self, id: &str) {
        if self.peers.get(id).is_some_and(|p| !p.paired && !p.found && p.link.is_none()) {
            self.peers.remove(id);
        }
    }
}

impl Net {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Copy the current state into the shared view and let the window and tray know.
    fn publish(&self) {
        let view = {
            let inner = self.lock();
            NetView {
                my_id: identity::short(&self.me),
                peers: inner
                    .peers
                    .iter()
                    .map(|(id, p)| PeerView {
                        id: id.clone(),
                        name: if p.name.is_empty() { "Unnamed PC".into() } else { p.name.clone() },
                        paired: p.paired,
                        connected: p.link.is_some(),
                        found: p.found,
                        addr: p
                            .link
                            .as_ref()
                            .map(|l| l.conn.remote_address().ip().to_string())
                            .or_else(|| p.addrs.first().map(|a| a.ip().to_string())),
                    })
                    .collect(),
                pairing: inner.pairing.as_ref().map(|p| PairingView {
                    id: p.id.clone(),
                    name: inner.name_of(&p.id),
                    code: p.code.clone(),
                    incoming: p.incoming,
                    confirmed: p.local_ok,
                }),
                connecting: inner.pending_pair.clone(),
                notice: inner.notice.clone(),
                controlling: inner.control.as_ref().map(|c| inner.name_of(&c.target)),
                controlled_by: inner.controlled_by.as_ref().map(|id| inner.name_of(id)),
            }
        };
        self.shared.set_net(view);
        (self.on_change)();
    }
}

async fn run(
    shared: Arc<Shared>,
    name: String,
    mut cmds: UnboundedReceiver<Cmd>,
    mut local: UnboundedReceiver<Local>,
    mut clips: UnboundedReceiver<Clip>,
    on_change: Arc<dyn Fn() + Send + Sync>,
) -> Result<(), String> {
    let me: Identity = identity::load_or_create()?;

    // Fixed port when free (easier firewall rules), any port otherwise (e.g. a dev second copy).
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT))
        .or_else(|_| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)))
        .map_err(|e| e.to_string())?;
    let port = socket.local_addr().map_err(|e| e.to_string())?.port();
    let mut endpoint = quinn::Endpoint::new(
        quinn::EndpointConfig::default(),
        Some(tls::server(&me)?),
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| e.to_string())?;
    endpoint.set_default_client_config(tls::client(&me)?);
    dlog!("this PC {} listening on UDP {port}", identity::short(&me.id));

    let mut inner = Inner { name, ..Default::default() };
    for p in peers::load() {
        inner.peers.insert(p.id, Peer { name: p.name, paired: true, ..Default::default() });
    }
    let net = Arc::new(Net { me: me.id.clone(), endpoint, shared, on_change, inner: Mutex::new(inner) });
    net.publish();

    // Announce this PC and watch for others.
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    announce(&mdns, &net, port)?;
    let browse = mdns.browse(SERVICE).map_err(|e| e.to_string())?;
    let (found_tx, mut found_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("mdns-events".into())
        .spawn(move || {
            while let Ok(event) = browse.recv() {
                if found_tx.send(event).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Incoming connections.
    {
        let net = net.clone();
        tokio::spawn(async move {
            while let Some(incoming) = net.endpoint.accept().await {
                let net = net.clone();
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        run_link(net, conn, false).await;
                    }
                });
            }
        });
    }

    let mut tick = tokio::time::interval(Duration::from_secs(3));
    loop {
        tokio::select! {
            Some(event) = local.recv() => on_local(&net, event),
            Some(clip) = clips.recv() => share_clip(&net, clip),
            Some(cmd) = cmds.recv() => on_command(&net, cmd),
            Some(event) = found_rx.recv() => on_discovery(&net, event),
            _ = tick.tick() => maintain(&net, &mdns, port),
        }
    }
}

fn announce(mdns: &ServiceDaemon, net: &Net, port: u16) -> Result<(), String> {
    let instance = format!("homekvm-{}", &net.me[..16]);
    let host = format!("{instance}.local.");
    let name = net.lock().name.clone();
    let props = [("id", net.me.as_str()), ("name", name.as_str()), ("v", "1")];
    let info = ServiceInfo::new(SERVICE, &instance, &host, "", port, &props[..])
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
    mdns.register(info).map_err(|e| e.to_string())
}

fn on_discovery(net: &Arc<Net>, event: ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let Some(id) = info.get_property_val_str("id").map(str::to_string) else { return };
            if id == net.me || id.len() != 64 {
                return;
            }
            let name = info.get_property_val_str("name").unwrap_or("").to_string();
            let port = info.get_port();
            let mut addrs: Vec<SocketAddr> = info
                .get_addresses()
                .iter()
                .filter(|ip| usable(ip))
                .map(|ip| SocketAddr::new(*ip, port))
                .collect();
            addrs.sort_by_key(preference);
            {
                let mut inner = net.lock();
                inner.mdns_names.insert(info.get_fullname().to_string(), id.clone());
                let peer = inner.peers.entry(id.clone()).or_default();
                peer.found = true;
                peer.addrs = addrs;
                dlog!("found {} at {:?}", identity::short(&id), peer.addrs);
                if !name.is_empty() {
                    peer.name = name;
                }
            }
            net.publish();
        }
        ServiceEvent::ServiceRemoved(_, fullname) => {
            {
                let mut inner = net.lock();
                if let Some(id) = inner.mdns_names.remove(&fullname) {
                    if let Some(peer) = inner.peers.get_mut(&id) {
                        peer.found = false;
                    }
                    inner.prune(&id);
                }
            }
            net.publish();
        }
        _ => {}
    }
}

/// Addresses worth dialing. The socket is IPv4, so IPv4 only, and no loopback.
fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_unspecified() && !v4.is_link_local(),
        IpAddr::V6(_) => false,
    }
}

/// Try typical home-network addresses first; 172.16/12 is often a virtual adapter
/// (WSL, Hyper-V, Docker) that another PC can't reach.
fn preference(addr: &SocketAddr) -> u8 {
    match addr.ip() {
        IpAddr::V4(v4) => match v4.octets() {
            [192, 168, ..] => 0,
            [10, ..] => 1,
            [172, b, ..] if (16..32).contains(&b) => 3,
            _ => 2,
        },
        IpAddr::V6(_) => 4,
    }
}

fn on_command(net: &Arc<Net>, cmd: Cmd) {
    match cmd {
        Cmd::Pair(id) => {
            let mut inner = net.lock();
            if inner.pairing.is_some() || inner.pending_pair.is_some() {
                return;
            }
            inner.notice = None;
            let conn = inner.peers.get(&id).and_then(|p| p.link.as_ref()).map(|l| l.conn.clone());
            match conn {
                Some(conn) => begin_pairing(net, &mut inner, &id, &conn),
                None => {
                    let addrs = inner.peers.get(&id).map(|p| p.addrs.clone()).unwrap_or_default();
                    inner.pending_pair = Some(id.clone());
                    drop(inner);
                    spawn_connect(net.clone(), id, addrs);
                }
            }
        }
        Cmd::Confirm(id, ok) => {
            let mut inner = net.lock();
            if inner.pairing.as_ref().is_none_or(|p| p.id != id) {
                return;
            }
            if ok {
                if let Some(p) = inner.pairing.as_mut() {
                    p.local_ok = true;
                }
                inner.send(&id, Msg::PairAccept);
                inner.finish_pairing_if_done();
            } else {
                inner.send(&id, Msg::PairReject);
                inner.pairing = None;
            }
        }
        Cmd::Forget(id) => {
            let mut inner = net.lock();
            inner.send(&id, Msg::Unpair);
            if let Some(peer) = inner.peers.get_mut(&id) {
                peer.paired = false;
                // Give the Unpair message a moment to go out, then hang up.
                if let Some(link) = &peer.link {
                    let conn = link.conn.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        conn.close(0u32.into(), b"forgotten");
                    });
                }
            }
            inner.save_paired();
            inner.prune(&id);
        }
        Cmd::Rename(name) => {
            let mut inner = net.lock();
            if inner.name != name {
                inner.name = name;
                inner.name_changed = true;
            }
        }
        Cmd::ClearNotice => net.lock().notice = None,
    }
    net.publish();
}

/// Show the code on this PC and ask the other PC to show it too.
fn begin_pairing(net: &Net, inner: &mut Inner, id: &str, conn: &quinn::Connection) {
    let Some(code) = tls::pairing_code(conn, &net.me, id) else {
        inner.notice = Some("Couldn't start pairing. Try again.".into());
        return;
    };
    inner.pairing =
        Some(Pairing { id: id.to_string(), code, incoming: false, local_ok: false, remote_ok: false, started: Instant::now() });
    inner.send(id, Msg::PairRequest);
}

fn spawn_connect(net: Arc<Net>, id: String, addrs: Vec<SocketAddr>) {
    if !net.lock().connecting.insert(id.clone()) {
        return;
    }
    tokio::spawn(async move {
        let mut connected = false;
        for addr in addrs {
            let Ok(connecting) = net.endpoint.connect(addr, "homekvm") else { continue };
            match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
                Ok(Ok(conn)) if tls::peer_id(&conn).as_deref() == Some(id.as_str()) => {
                    connected = true;
                    net.lock().connecting.remove(&id);
                    run_link(net.clone(), conn, true).await;
                    break;
                }
                // Someone else answered at that address: not the PC we wanted.
                Ok(Ok(conn)) => conn.close(0u32.into(), b"wrong peer"),
                _ => {}
            }
        }
        if !connected {
            let mut inner = net.lock();
            inner.connecting.remove(&id);
            if inner.pending_pair.as_deref() == Some(id.as_str()) {
                inner.pending_pair = None;
                inner.notice = Some(format!(
                    "Couldn't reach {}. Check that both PCs are on the same network and HomeKVM is allowed through the firewall.",
                    inner.name_of(&id)
                ));
                drop(inner);
                net.publish();
            }
        }
    });
}

/// One connection to another PC, from handshake to hang-up.
async fn run_link(net: Arc<Net>, conn: quinn::Connection, dialed: bool) {
    let Some(id) = tls::peer_id(&conn) else {
        conn.close(0u32.into(), b"no identity");
        return;
    };
    if id == net.me {
        conn.close(0u32.into(), b"self");
        return;
    }

    // The dialing side opens the control stream (it appears once Hello is written).
    let streams = if dialed {
        tokio::time::timeout(CONNECT_TIMEOUT, conn.open_bi()).await
    } else {
        tokio::time::timeout(Duration::from_secs(15), conn.accept_bi()).await
    };
    let Ok(Ok((mut send, mut recv))) = streams else {
        conn.close(0u32.into(), b"no stream");
        return;
    };

    let (tx, mut rx) = unbounded_channel::<Msg>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if proto::write(&mut send, &msg).await.is_err() {
                break;
            }
        }
    });

    let link_id = conn.stable_id();
    {
        let mut inner = net.lock();
        let my_name = inner.name.clone();
        let peer = inner.peers.entry(id.clone()).or_default();
        if let Some(old) = peer.link.as_ref().map(|l| l.conn.clone()) {
            // Two links to the same PC (both dialed at once): both sides keep the one
            // dialed by the PC with the smaller ID.
            let dialer = if dialed { &net.me } else { &id };
            if dialer != std::cmp::min(&net.me, &id) {
                drop(inner);
                writer.abort();
                conn.close(0u32.into(), b"duplicate");
                return;
            }
            old.close(0u32.into(), b"duplicate");
        }
        if peer.addrs.is_empty() {
            peer.addrs = vec![conn.remote_address()];
        }
        let paired = peer.paired;
        peer.link = Some(Link { conn: conn.clone(), tx: tx.clone(), since: Instant::now() });
        let _ = tx.send(Msg::Hello { name: my_name, paired });
        dlog!("link up with {} ({})", identity::short(&id), if dialed { "dialed" } else { "incoming" });

        if inner.pending_pair.as_deref() == Some(id.as_str()) {
            inner.pending_pair = None;
            begin_pairing(&net, &mut inner, &id, &conn);
        }
    }
    net.publish();

    // Clipboard copies from this PC arrive on their own streams.
    let clip_reader = {
        let (net, conn, id) = (net.clone(), conn.clone(), id.clone());
        tokio::spawn(async move {
            while let Ok(mut stream) = conn.accept_uni().await {
                let result = proto::read_clip(&mut stream).await;
                let trusted = net.lock().peers.get(&id).is_some_and(|p| p.paired);
                match result {
                    Ok(clip) if trusted => {
                        dlog!(
                            "clipboard from {}: text {}, image {} bytes",
                            identity::short(&id),
                            clip.text.as_ref().map_or(0, |t| t.len()),
                            clip.image.as_ref().map_or(0, |i| i.len())
                        );
                        clipboard::apply(clip);
                    }
                    Ok(_) => {}
                    Err(e) => dlog!("clipboard from {}: {e}", identity::short(&id)),
                }
            }
        })
    };

    while let Ok(msg) = proto::read(&mut recv).await {
        on_message(&net, &id, &conn, msg);
    }

    clip_reader.abort();
    writer.abort();
    dlog!("link down with {}", identity::short(&id));
    {
        let mut inner = net.lock();
        let mine = inner.peers.get(&id).and_then(|p| p.link.as_ref()).is_some_and(|l| l.conn.stable_id() == link_id);
        if mine {
            if let Some(peer) = inner.peers.get_mut(&id) {
                peer.link = None;
            }
            // A dropped connection never leaves the keyboard or mouse stuck elsewhere.
            if inner.control.as_ref().is_some_and(|c| c.target == id) {
                inner.control = None;
                input::release(None);
            }
            if inner.controlled_by.as_deref() == Some(id.as_str()) {
                inner.end_controlled();
            }
            if inner.pairing.as_ref().is_some_and(|p| p.id == id) {
                inner.pairing = None;
                inner.notice = Some(format!("Lost the connection to {} while pairing. Try again.", inner.name_of(&id)));
            }
        }
        inner.prune(&id);
    }
    net.publish();
}

fn on_message(net: &Net, id: &str, conn: &quinn::Connection, msg: Msg) {
    // Mouse moves, keys and buttons arrive hundreds of times a second: handle them
    // without redrawing the window.
    let msg = match on_input_message(net, id, msg) {
        Some(other) => other,
        None => return,
    };
    let mut open_window = false;
    {
        let mut inner = net.lock();
        let Some(peer) = inner.peers.get_mut(id) else { return };
        match msg {
            Msg::Hello { name, paired } => {
                let renamed = !name.is_empty() && peer.name != name;
                if !name.is_empty() {
                    peer.name = name;
                }
                let we_trust = peer.paired;
                if we_trust && !paired {
                    // The other PC forgot us; trust has to be mutual.
                    peer.paired = false;
                    inner.notice =
                        Some(format!("{} no longer trusts this PC. Pair again to reconnect.", inner.name_of(id)));
                    inner.save_paired();
                } else if !we_trust && paired && inner.pairing.is_none() {
                    // It trusts us but we don't know it (we forgot it): tell it.
                    inner.send(id, Msg::Unpair);
                } else if we_trust && renamed {
                    inner.save_paired();
                }
            }
            Msg::PairRequest => {
                if inner.pairing.is_some() {
                    inner.send(id, Msg::PairReject);
                } else if let Some(code) = tls::pairing_code(conn, &net.me, id) {
                    inner.pairing = Some(Pairing {
                        id: id.to_string(),
                        code,
                        incoming: true,
                        local_ok: false,
                        remote_ok: false,
                        started: Instant::now(),
                    });
                    inner.notice = None;
                    open_window = true;
                }
            }
            Msg::PairAccept => {
                if let Some(p) = inner.pairing.as_mut().filter(|p| p.id == id) {
                    p.remote_ok = true;
                    inner.finish_pairing_if_done();
                }
            }
            Msg::PairReject => {
                if inner.pairing.as_ref().is_some_and(|p| p.id == id) {
                    inner.pairing = None;
                    inner.notice = Some(format!("{} cancelled pairing.", inner.name_of(id)));
                }
            }
            Msg::Unpair => {
                if peer.paired {
                    peer.paired = false;
                    inner.notice = Some(format!("{} removed this PC. Pair again to reconnect.", inner.name_of(id)));
                    inner.save_paired();
                }
            }
            // Keyboard and mouse messages were handled in on_input_message.
            _ => {}
        }
    }
    if open_window {
        win::show_main_window();
    }
    net.publish();
}

/// Keyboard and mouse sharing messages. Returns other messages untouched.
fn on_input_message(net: &Net, id: &str, msg: Msg) -> Option<Msg> {
    let mut inner = net.lock();
    // Only trusted PCs may move this PC's mouse or type on it.
    let trusted = inner.peers.get(id).is_some_and(|p| p.paired);
    let from_controller = inner.controlled_by.as_deref() == Some(id);
    let to_target = inner.control.as_ref().is_some_and(|c| c.target == id);
    let mut changed = false;

    match msg {
        Msg::Move { dx, dy } if from_controller => {
            if let Some((dir, y)) = input::inject_move(dx, dy) {
                inner.send(id, Msg::Overflow { dir, y });
            }
        }
        Msg::Button { b, down } if from_controller => input::inject_button(b, down),
        Msg::Wheel { d, h } if from_controller => input::inject_wheel(d, h),
        Msg::Key { vk, sc, ext, down } if from_controller => input::inject_key(vk, sc, ext, down),
        Msg::Move { .. } | Msg::Button { .. } | Msg::Wheel { .. } | Msg::Key { .. } => {}

        Msg::Enter { edge, y } => {
            if !trusted || !input::available() || net.shared.paused() {
                // Can't or won't be controlled right now: send the mouse back.
                inner.send(id, Msg::TakeOver);
            } else {
                // Someone else's mouse arriving here wins over our own outgoing control.
                inner.stop_control(None);
                if inner.controlled_by.as_deref() != Some(id) {
                    inner.end_controlled();
                }
                inner.controlled_by = Some(id.to_string());
                input::set_controlled(true);
                input::inject_enter(edge, y);
                changed = true;
            }
        }
        Msg::Leave => {
            if from_controller {
                inner.end_controlled();
                changed = true;
            }
        }
        Msg::Overflow { dir, y } => {
            if to_target {
                let from = inner.control.as_ref().map(|c| c.pos).unwrap_or(0);
                let pos = from + dir as i32;
                if pos == 0 {
                    // Back home: the cursor appears at the edge facing that PC.
                    let edge = if from > 0 { Edge::Right } else { Edge::Left };
                    inner.stop_control(Some((edge, y)));
                } else if let Some(next) = peer_at(&net.shared.layout(), pos).filter(|n| inner.online(n)) {
                    // On along the row to the next PC.
                    inner.send(id, Msg::Leave);
                    let edge = if dir > 0 { Edge::Left } else { Edge::Right };
                    inner.send(&next, Msg::Enter { edge, y });
                    inner.control = Some(Control { target: next, pos });
                }
                changed = true;
            }
        }
        Msg::TakeOver => {
            if to_target {
                inner.stop_control(None);
                changed = true;
            }
        }
        other => return Some(other),
    }
    drop(inner);
    if changed {
        net.publish();
    }
    None
}

/// A copy made on this PC: send it to every connected paired PC. Last copy wins.
fn share_clip(net: &Arc<Net>, clip: Clip) {
    let conns: Vec<(String, quinn::Connection)> = net
        .lock()
        .peers
        .iter()
        .filter(|(_, p)| p.paired)
        .filter_map(|(id, p)| p.link.as_ref().map(|l| (id.clone(), l.conn.clone())))
        .collect();
    let clip = Arc::new(clip);
    for (id, conn) in conns {
        let clip = clip.clone();
        tokio::spawn(async move {
            if let Err(e) = proto::send_clip(&conn, &clip).await {
                dlog!("clipboard to {}: {e}", identity::short(&id));
            }
        });
    }
}

/// Events from this PC's own keyboard and mouse hooks.
fn on_local(net: &Arc<Net>, event: Local) {
    let mut inner = net.lock();
    let target = inner.control.as_ref().map(|c| c.target.clone());
    let mut changed = false;
    match event {
        Local::Move { dx, dy } => {
            if let Some(t) = &target {
                inner.send(t, Msg::Move { dx, dy });
            }
        }
        Local::Button { b, down } => {
            if let Some(t) = &target {
                inner.send(t, Msg::Button { b, down });
            }
        }
        Local::Wheel { d, h } => {
            if let Some(t) = &target {
                inner.send(t, Msg::Wheel { d, h });
            }
        }
        Local::Key { vk, sc, ext, down } => {
            if let Some(t) = &target {
                inner.send(t, Msg::Key { vk, sc, ext, down });
            }
        }
        Local::EdgePush { dir, y } => {
            let busy = target.is_some() || inner.controlled_by.is_some() || net.shared.paused();
            let next = peer_at(&net.shared.layout(), dir as i32).filter(|id| inner.online(id));
            if let (false, Some(id)) = (busy, next) {
                let edge = if dir > 0 { Edge::Left } else { Edge::Right };
                inner.control = Some(Control { target: id.clone(), pos: dir as i32 });
                input::capture();
                inner.send(&id, Msg::Enter { edge, y });
                changed = true;
            }
        }
        Local::Release => {
            if target.is_some() {
                inner.stop_control(None);
                changed = true;
            }
        }
        Local::Activity => {
            // Someone is using this PC's own keyboard or mouse: they take over.
            if let Some(id) = inner.controlled_by.clone() {
                inner.send(&id, Msg::TakeOver);
                inner.end_controlled();
                changed = true;
            }
        }
    }
    drop(inner);
    if changed {
        net.publish();
    }
}

/// Every few seconds: timeouts, reconnecting paired PCs, and name changes.
fn maintain(net: &Arc<Net>, mdns: &ServiceDaemon, port: u16) {
    let mut dial = Vec::new();
    let mut announce_again = false;
    let mut changed = false;
    {
        let mut inner = net.lock();

        if let Some(p) = inner.pairing.as_ref().filter(|p| p.started.elapsed() > PAIRING_TIMEOUT) {
            let id = p.id.clone();
            inner.send(&id, Msg::PairReject);
            inner.pairing = None;
            inner.notice = Some("Pairing timed out. Try again.".into());
            changed = true;
        }

        let pairing_with = inner.pairing.as_ref().map(|p| p.id.clone());
        for (id, peer) in &inner.peers {
            match &peer.link {
                Some(link) => {
                    let idle = !peer.paired && pairing_with.as_deref() != Some(id.as_str()) && link.since.elapsed() > UNPAIRED_IDLE;
                    if idle {
                        link.conn.close(0u32.into(), b"idle");
                    }
                }
                None => {
                    if peer.paired && peer.found && net.me < *id && !peer.addrs.is_empty() {
                        dial.push((id.clone(), peer.addrs.clone()));
                    }
                }
            }
        }

        if inner.name_changed {
            inner.name_changed = false;
            announce_again = true;
            let name = inner.name.clone();
            let ids: Vec<(String, bool)> =
                inner.peers.iter().filter(|(_, p)| p.link.is_some()).map(|(id, p)| (id.clone(), p.paired)).collect();
            for (id, paired) in ids {
                inner.send(&id, Msg::Hello { name: name.clone(), paired });
            }
        }
    }
    if announce_again {
        if let Err(e) = announce(mdns, net, port) {
            eprintln!("mDNS announce: {e}");
        }
    }
    for (id, addrs) in dial {
        spawn_connect(net.clone(), id, addrs);
    }
    if changed {
        net.publish();
    }
}
