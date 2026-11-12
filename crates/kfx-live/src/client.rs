//! A client that lives on its own thread and talks to the UI through channels.
//!
//! * Connects with a timeout, reconnects forever (every [`Config::reconnect_interval`]) and
//!   re-sends the subscriptions it knows after a reconnect (the game forgets them when the
//!   connection drops, `src/api.c:1680`). Without a game it just keeps trying quietly: every request
//!   then fails fast with [`ClientError::NotConnected`].
//! * One request is in flight at a time (the game reads one `recv` per turn and a request split by
//!   the 4095 byte buffer is lost, see [`crate::proto`]); further requests queue. A request that
//!   gets no answer within [`Config::request_timeout`] fails with [`ClientError::Timeout`] and the
//!   next one is sent.
//! * Replies are matched by an `ack` number the client adds. Replies of requests sent with
//!   [`LiveClient::call`] go to the caller, all others arrive as [`LiveEvent::Reply`] from
//!   [`LiveClient::poll`]. Events and variable updates arrive as [`LiveEvent::Event`].
use crate::proto::{self, GameEvent, Incoming, Reply, Request};
use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type RequestId = u64;

#[derive(Clone, Debug)]
pub struct Config {
    pub addr: SocketAddr,
    pub connect_timeout: Duration,
    pub reconnect_interval: Duration,
    pub request_timeout: Duration,
    /// Emit [`LiveEvent::Wire`] for every line sent / received.
    pub log_wire: bool,
}

impl Config {
    pub fn localhost(port: u16) -> Config {
        Config {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            connect_timeout: Duration::from_millis(500),
            reconnect_interval: Duration::from_millis(1000),
            request_timeout: Duration::from_secs(5),
            log_wire: false,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::localhost(proto::DEFAULT_PORT)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("not connected to a game")]
    NotConnected,
    #[error("the game did not answer in time")]
    Timeout,
    #[error("the connection was lost")]
    Disconnected,
    #[error("the client was shut down")]
    Shutdown,
    #[error("{0}")]
    Proto(#[from] proto::ProtoError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected,
}

/// What the status line shows.
#[derive(Clone, Debug)]
pub struct Status {
    pub state: ConnState,
    pub addr: SocketAddr,
    pub kfx_version: Option<String>,
    pub last_error: Option<String>,
    pub connected_since: Option<Instant>,
    pub requests_sent: u64,
    pub replies: u64,
    pub events: u64,
    pub timeouts: u64,
    pub queued: usize,
}

#[derive(Clone, Debug)]
pub enum LiveEvent {
    Connected {
        kfx_version: Option<String>,
    },
    Disconnected {
        reason: String,
    },
    /// Answer (or failure) of a request sent with [`LiveClient::send`].
    Reply {
        id: RequestId,
        request: Request,
        result: Result<Reply, ClientError>,
    },
    Event(GameEvent),
    /// A reply whose request already timed out.
    LateReply(Reply),
    /// A line that is not JSON the protocol knows.
    Garbage(String),
    /// Raw traffic (only with [`Config::log_wire`]): `sent` is true for client to game.
    Wire {
        sent: bool,
        text: String,
    },
}

type Waiter = Sender<Result<Reply, ClientError>>;

enum Cmd {
    Send {
        id: RequestId,
        req: Request,
        waiter: Option<Waiter>,
    },
    Shutdown,
}

pub struct LiveClient {
    tx: Sender<Cmd>,
    rx: Receiver<LiveEvent>,
    status: Arc<Mutex<Status>>,
    next_id: std::sync::atomic::AtomicU64,
    join: Option<JoinHandle<()>>,
    timeout: Duration,
}

impl LiveClient {
    /// Start the client thread. Never blocks and never fails: no game is not an error.
    pub fn spawn(cfg: Config) -> LiveClient {
        let (tx, cmd_rx) = mpsc::channel();
        let (ev_tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status {
            state: ConnState::Disconnected,
            addr: cfg.addr,
            kfx_version: None,
            last_error: None,
            connected_since: None,
            requests_sent: 0,
            replies: 0,
            events: 0,
            timeouts: 0,
            queued: 0,
        }));
        let st = status.clone();
        let timeout = cfg.request_timeout;
        let join = std::thread::Builder::new()
            .name("kfx-live".into())
            .spawn(move || Worker::new(cfg, cmd_rx, ev_tx, st).run())
            .expect("spawn kfx-live thread");
        LiveClient {
            tx,
            rx,
            status,
            next_id: std::sync::atomic::AtomicU64::new(1),
            join: Some(join),
            timeout,
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn is_connected(&self) -> bool {
        self.status().state == ConnState::Connected
    }

    /// Queue a request; the answer arrives as [`LiveEvent::Reply`] with the returned id.
    pub fn send(&self, req: Request) -> RequestId {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Send {
            id,
            req,
            waiter: None,
        });
        id
    }

    /// Send and wait for the answer (for tools and tests; the UI uses [`LiveClient::send`]).
    pub fn call(&self, req: Request) -> Result<Reply, ClientError> {
        self.call_timeout(req, self.timeout + Duration::from_secs(2))
    }

    pub fn call_timeout(&self, req: Request, timeout: Duration) -> Result<Reply, ClientError> {
        let (wtx, wrx) = mpsc::channel();
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self
            .tx
            .send(Cmd::Send {
                id,
                req,
                waiter: Some(wtx),
            })
            .is_err()
        {
            return Err(ClientError::Shutdown);
        }
        match wrx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(ClientError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(ClientError::Shutdown),
        }
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> Vec<LiveEvent> {
        self.rx.try_iter().collect()
    }

    /// Wait for the next event for at most `t`.
    pub fn poll_wait(&self, t: Duration) -> Option<LiveEvent> {
        self.rx.recv_timeout(t).ok()
    }

    /// Wait until connected (the handshake is done), up to `t`.
    pub fn wait_connected(&self, t: Duration) -> bool {
        let end = Instant::now() + t;
        while Instant::now() < end {
            if self.is_connected() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.is_connected()
    }

    /// Close the connection (a clean TCP shutdown, so the game logs "API connection closed") and
    /// stop the thread.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for LiveClient {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Pending {
    id: RequestId,
    req: Request,
    waiter: Option<Waiter>,
    sent_at: Instant,
    internal: bool,
}

struct Worker {
    cfg: Config,
    cmds: Receiver<Cmd>,
    events: Sender<LiveEvent>,
    status: Arc<Mutex<Status>>,
    stream: Option<TcpStream>,
    inbuf: Vec<u8>,
    queue: VecDeque<(RequestId, Request, Option<Waiter>, bool)>,
    in_flight: Option<Pending>,
    sub_events: BTreeSet<String>,
    sub_vars: BTreeSet<(String, Option<proto::Player>)>,
    ack_counter: u64,
    shutting_down: bool,
}

impl Worker {
    fn new(
        cfg: Config,
        cmds: Receiver<Cmd>,
        events: Sender<LiveEvent>,
        status: Arc<Mutex<Status>>,
    ) -> Worker {
        Worker {
            cfg,
            cmds,
            events,
            status,
            stream: None,
            inbuf: vec![],
            queue: VecDeque::new(),
            in_flight: None,
            sub_events: BTreeSet::new(),
            sub_vars: BTreeSet::new(),
            // internal ids never collide with the handle's and stay inside an i32
            ack_counter: 1 << 30,
            shutting_down: false,
        }
    }

    fn st(&self, f: impl FnOnce(&mut Status)) {
        f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }

    fn emit(&self, e: LiveEvent) {
        let _ = self.events.send(e);
    }

    fn drain_cmds(&mut self) {
        while let Ok(c) = self.cmds.try_recv() {
            match c {
                Cmd::Shutdown => self.shutting_down = true,
                Cmd::Send { id, req, waiter } => {
                    if self.stream.is_none() {
                        self.fail(id, req, waiter, ClientError::NotConnected);
                    } else {
                        self.queue.push_back((id, req, waiter, false));
                    }
                }
            }
        }
        let n = self.queue.len();
        self.st(|s| s.queued = n);
    }

    fn fail(&self, id: RequestId, req: Request, waiter: Option<Waiter>, err: ClientError) {
        match waiter {
            Some(w) => {
                let _ = w.send(Err(err));
            }
            None => self.emit(LiveEvent::Reply {
                id,
                request: req,
                result: Err(err),
            }),
        }
    }

    fn run(mut self) {
        while !self.shutting_down {
            if self.stream.is_none() {
                self.try_connect();
                continue;
            }
            self.drain_cmds();
            if self.shutting_down {
                break;
            }
            self.send_next();
            self.read_some();
            self.check_timeout();
        }
        self.disconnect("client stopped", true);
        // fail whatever is still queued
        while let Some((id, req, w, internal)) = self.queue.pop_front() {
            if !internal {
                self.fail(id, req, w, ClientError::Shutdown);
            }
        }
    }

    fn try_connect(&mut self) {
        self.st(|s| s.state = ConnState::Connecting);
        match TcpStream::connect_timeout(&self.cfg.addr, self.cfg.connect_timeout) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                let _ = s.set_read_timeout(Some(Duration::from_millis(25)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
                self.stream = Some(s);
                self.inbuf.clear();
                // handshake + re-subscribe, ahead of anything the user queued
                let mut pre: Vec<Request> = vec![Request::GetKfxInfo];
                pre.extend(
                    self.sub_events
                        .iter()
                        .map(|e| Request::SubscribeEvent(e.clone())),
                );
                pre.extend(self.sub_vars.iter().map(|(v, p)| Request::SubscribeVar {
                    var: v.clone(),
                    player: p.clone(),
                }));
                for r in pre.into_iter().rev() {
                    self.ack_counter += 1;
                    self.queue.push_front((self.ack_counter, r, None, true));
                }
                self.st(|s| {
                    s.state = ConnState::Connected;
                    s.connected_since = Some(Instant::now());
                    s.last_error = None;
                });
            }
            Err(e) => {
                self.st(|s| {
                    s.state = ConnState::Disconnected;
                    s.last_error = Some(e.to_string());
                });
                // wait, but stay responsive to commands and shutdown
                let end = Instant::now() + self.cfg.reconnect_interval;
                while Instant::now() < end && !self.shutting_down {
                    match self.cmds.recv_timeout(Duration::from_millis(25)) {
                        Ok(Cmd::Shutdown) => self.shutting_down = true,
                        Ok(Cmd::Send { id, req, waiter }) => {
                            self.fail(id, req, waiter, ClientError::NotConnected)
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => self.shutting_down = true,
                    }
                }
            }
        }
    }

    fn send_next(&mut self) {
        if self.in_flight.is_some() {
            return;
        }
        let Some((id, req, waiter, internal)) = self.queue.pop_front() else {
            return;
        };
        let ack = if internal { id } else { id };
        let bytes = match proto::encode(&req, Some(ack)) {
            Ok(b) => b,
            Err(e) => {
                self.fail(id, req, waiter, e.into());
                return;
            }
        };
        let mut line = bytes.clone();
        line.push(b'\n');
        let ok = self
            .stream
            .as_mut()
            .map(|s| s.write_all(&line).and_then(|_| s.flush()))
            .map(|r| r.is_ok())
            .unwrap_or(false);
        if !ok {
            self.fail(id, req, waiter, ClientError::Disconnected);
            self.disconnect("write failed", false);
            return;
        }
        if self.cfg.log_wire {
            self.emit(LiveEvent::Wire {
                sent: true,
                text: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        self.st(|s| s.requests_sent += 1);
        self.in_flight = Some(Pending {
            id,
            req,
            waiter,
            sent_at: Instant::now(),
            internal,
        });
    }

    fn read_some(&mut self) {
        let Some(s) = self.stream.as_mut() else {
            return;
        };
        let mut buf = [0u8; 8192];
        match s.read(&mut buf) {
            Ok(0) => self.disconnect("the game closed the connection", false),
            Ok(n) => {
                self.inbuf.extend_from_slice(&buf[..n]);
                while let Some(p) = self.inbuf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = self.inbuf.drain(..=p).collect();
                    let text = String::from_utf8_lossy(&line[..line.len() - 1])
                        .trim()
                        .to_string();
                    if !text.is_empty() {
                        self.handle_line(&text);
                    }
                }
                if self.inbuf.len() > 1 << 20 {
                    self.inbuf.clear();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => self.disconnect(&e.to_string(), false),
        }
    }

    fn handle_line(&mut self, text: &str) {
        if self.cfg.log_wire {
            self.emit(LiveEvent::Wire {
                sent: false,
                text: text.to_string(),
            });
        }
        match proto::parse_incoming(text) {
            Ok(Incoming::Reply(r)) => self.handle_reply(r),
            Ok(Incoming::Event(e)) => {
                self.st(|s| s.events += 1);
                self.emit(LiveEvent::Event(e));
            }
            Ok(Incoming::Other(_)) | Err(_) => self.emit(LiveEvent::Garbage(text.to_string())),
        }
    }

    fn handle_reply(&mut self, r: Reply) {
        let matches = match (&self.in_flight, r.ack) {
            (Some(p), Some(a)) => p.id == a,
            // errors raised before the server read the ack (INVALID_JSON ...) carry none
            (Some(_), None) => true,
            (None, _) => false,
        };
        if !matches {
            self.emit(LiveEvent::LateReply(r));
            return;
        }
        let p = self.in_flight.take().unwrap();
        self.st(|s| s.replies += 1);
        // remember subscriptions so they survive a reconnect
        if r.success {
            match &p.req {
                Request::SubscribeEvent(e) => {
                    self.sub_events.insert(e.clone());
                }
                Request::UnsubscribeEvent(e) => {
                    self.sub_events.remove(e);
                }
                Request::SubscribeVar { var, player } => {
                    self.sub_vars.insert((var.clone(), player.clone()));
                }
                Request::UnsubscribeVar { var, player } => {
                    self.sub_vars.remove(&(var.clone(), player.clone()));
                }
                Request::UnsubscribeAll => {
                    self.sub_events.clear();
                    self.sub_vars.clear();
                }
                Request::GetKfxInfo if p.internal => {
                    let v = r
                        .data
                        .as_ref()
                        .and_then(|d| d.get("kfx_version"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    self.st(|s| s.kfx_version = v.clone());
                    self.emit(LiveEvent::Connected { kfx_version: v });
                }
                _ => {}
            }
        }
        if p.internal {
            return;
        }
        match p.waiter {
            Some(w) => {
                let _ = w.send(Ok(r));
            }
            None => self.emit(LiveEvent::Reply {
                id: p.id,
                request: p.req,
                result: Ok(r),
            }),
        }
    }

    fn check_timeout(&mut self) {
        let expired = self
            .in_flight
            .as_ref()
            .is_some_and(|p| p.sent_at.elapsed() > self.cfg.request_timeout);
        if expired {
            let p = self.in_flight.take().unwrap();
            self.st(|s| s.timeouts += 1);
            if p.internal {
                // the handshake got no answer: still announce the link
                if matches!(p.req, Request::GetKfxInfo) {
                    self.emit(LiveEvent::Connected { kfx_version: None });
                }
            } else {
                self.fail(p.id, p.req, p.waiter, ClientError::Timeout);
            }
        }
    }

    fn disconnect(&mut self, reason: &str, graceful: bool) {
        if let Some(s) = self.stream.take() {
            if graceful {
                let _ = s.shutdown(Shutdown::Both);
            }
            self.st(|st| {
                st.state = ConnState::Disconnected;
                st.connected_since = None;
                st.last_error = Some(reason.to_string());
            });
            self.emit(LiveEvent::Disconnected {
                reason: reason.to_string(),
            });
        }
        if let Some(p) = self.in_flight.take() {
            if !p.internal {
                self.fail(p.id, p.req, p.waiter, ClientError::Disconnected);
            }
        }
        let keep: Vec<_> = self.queue.drain(..).collect();
        for (id, req, w, internal) in keep {
            if !internal {
                self.fail(id, req, w, ClientError::Disconnected);
            }
        }
        self.st(|s| s.queued = 0);
    }
}
