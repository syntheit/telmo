//! State and key handling for telmo-net. All drawing is in `ui.rs`.

use crate::backend::{Cmd, Event, Tx, mock};
use crate::model::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use qrcode::QrCode;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{SystemTime, UNIX_EPOCH};
use telmo_kit::{Flow, hits::Hits, input::TextInput, widgets::Toast};
use telmo_speed::{Phase, Record, Update};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Pane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Speed,
}

/// What the right-hand pane shows. Indexes point into the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Wifi,
    Wired(usize),
    Vpn,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiRow {
    Net(usize),
    /// A saved network that is out of range.
    Away(usize),
}

pub enum Dialog {
    Join(Join),
    Details(DetailsDialog),
    Ipv4(Ipv4Form),
    Dns(DnsForm),
    Help,
}

pub struct Join {
    pub network: String,
    pub title: String,
    pub input: TextInput,
}

pub struct DetailsDialog {
    pub network: Network,
    /// Known once revealed.
    pub password: Option<String>,
    pub confirm_forget: bool,
    /// Open networks have no password to reveal: `y` shows the QR directly.
    pub open_qr: bool,
}

impl DetailsDialog {
    /// The QR code to show next to the password, once it is revealed.
    pub fn qr(&self) -> Option<Qr> {
        let open = self.network.security == Security::Open && self.open_qr;
        match (&self.password, open) {
            (Some(password), _) => Some(Qr::new(&self.network, password)),
            (None, true) => Some(Qr::new(&self.network, "")),
            (None, false) => None,
        }
    }
}

/// Something a click can mean, recorded while drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// A row of the sidebar or the pane (interface or row index).
    Row(Focus, usize),
    /// A pane outside its rows.
    Pane(Focus),
    /// An item of the key bar.
    Key(KeyCode),
    /// A hint inside the open dialog.
    DialogKey(KeyCode),
    /// The open dialog itself.
    Inside,
    /// Anywhere else while a dialog is open.
    Outside,
}

pub struct Qr {
    pub payload: String,
}

pub struct Ipv4Form {
    pub interface: String,
    pub name: String,
    pub manual: bool,
    /// 0 is the mode toggle, then address, subnet, router.
    pub focus: usize,
    pub address: TextInput,
    pub subnet: TextInput,
    pub router: TextInput,
    dns: Vec<String>,
    pub error: Option<String>,
}

pub struct DnsForm {
    pub interface: String,
    pub name: String,
    pub input: TextInput,
    manual: Option<ManualIpv4>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct Meter {
    pub mbps: f64,
    pub samples: Vec<f64>,
    pub fraction: f32,
    pub done: bool,
}

#[derive(Default)]
pub struct Speed {
    pub running: bool,
    pub phase: Option<Phase>,
    pub down: Meter,
    pub up: Meter,
    pub idle_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub loaded_ms: Option<f64>,
    pub last: Option<Record>,
}

impl Speed {
    fn meter(&mut self, phase: Phase) -> Option<&mut Meter> {
        match phase {
            Phase::Download => Some(&mut self.down),
            Phase::Upload => Some(&mut self.up),
            Phase::Latency => None,
        }
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Server { .. } => {}
            Update::Latency { idle_ms, jitter_ms } => {
                self.idle_ms = Some(idle_ms);
                self.jitter_ms = Some(jitter_ms);
            }
            Update::Progress {
                phase,
                mbps,
                fraction,
            } => {
                self.phase = Some(phase);
                if let Some(meter) = self.meter(phase) {
                    meter.mbps = mbps;
                    meter.fraction = fraction;
                    meter.samples.push(mbps);
                }
            }
            Update::Result {
                phase,
                mbps,
                loaded_ms,
            } => {
                if let Some(meter) = self.meter(phase) {
                    meter.mbps = mbps;
                    meter.fraction = 1.0;
                    meter.done = true;
                }
                self.loaded_ms = loaded_ms.or(self.loaded_ms);
            }
            Update::Finished | Update::Failed(_) => {
                self.running = false;
                self.phase = None;
            }
        }
    }

    pub fn finished(&self) -> bool {
        self.down.done && self.up.done
    }

    /// The one-line result copied with `c`.
    pub fn summary(&self) -> String {
        format!(
            "↓ {:.1} Mbps  ↑ {:.1} Mbps  {:.0} ms",
            self.down.mbps,
            self.up.mbps,
            self.idle_ms.unwrap_or(0.0)
        )
    }
}

pub struct App {
    pub snapshot: Snapshot,
    pub focus: Focus,
    pub screen: Screen,
    /// Sidebar selection: an interface, or the VPN entry after them.
    pub sel: usize,
    /// Selected row in the pane.
    pub row: usize,
    pub show_away: bool,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    /// Ids of rows waiting for a backend `Done`.
    pub pending: HashSet<String>,
    pub details: Option<Details>,
    pub speed: Speed,
    pub tick: u64,
    pub hits: Hits<Click>,
    cmds: UnboundedSender<Cmd>,
    events: Tx,
    mock: bool,
}

impl App {
    pub fn new(snapshot: Snapshot, cmds: UnboundedSender<Cmd>, events: Tx, mock: bool) -> Self {
        let sel = snapshot
            .interfaces
            .iter()
            .position(|i| i.kind == InterfaceKind::Wifi)
            .unwrap_or(0);
        Self {
            snapshot,
            focus: Focus::Pane,
            screen: Screen::Main,
            sel,
            row: 0,
            show_away: false,
            dialog: None,
            toast: None,
            pending: HashSet::new(),
            details: None,
            speed: Speed::default(),
            tick: 0,
            hits: Hits::default(),
            cmds,
            events,
            mock,
        }
    }

    // --- queries used by the UI ---

    pub fn pane(&self) -> Pane {
        match self.snapshot.interfaces.get(self.sel) {
            Some(i) if i.kind == InterfaceKind::Wifi && self.snapshot.wifi.is_some() => Pane::Wifi,
            Some(_) => Pane::Wired(self.sel),
            None if self.sel == self.snapshot.interfaces.len() => Pane::Vpn,
            None => Pane::Empty,
        }
    }

    pub fn wifi_rows(&self) -> Vec<WifiRow> {
        let Some(wifi) = &self.snapshot.wifi else {
            return vec![];
        };
        // Connected first, then saved, then the rest; nameless last, strongest first.
        let mut order: Vec<usize> = (0..wifi.networks.len()).collect();
        order.sort_by_key(|&i| {
            let n = &wifi.networks[i];
            (
                !n.connected,
                !n.saved,
                n.ssid.is_none(),
                std::cmp::Reverse(n.strength),
                n.ssid.clone(),
            )
        });
        let mut rows: Vec<WifiRow> = order.into_iter().map(WifiRow::Net).collect();
        if self.show_away {
            rows.extend((0..wifi.saved_elsewhere.len()).map(WifiRow::Away));
        }
        rows
    }

    /// Whether the settings rows (IPv4, DNS) are offered.
    pub fn has_settings(&self, interface: &Interface) -> bool {
        self.snapshot.caps.edit_ip && interface.config.is_some()
    }

    pub fn wifi_ipv4(&self) -> Option<&Ipv4> {
        let id = &self.snapshot.wifi.as_ref()?.interface;
        self.snapshot
            .interfaces
            .iter()
            .find(|i| &i.id == id)?
            .ipv4
            .as_ref()
    }

    /// Whether the system joins this saved network by itself, if known.
    pub fn auto_join(&self, ssid: &str) -> Option<bool> {
        let wifi = self.snapshot.wifi.as_ref()?;
        let network = wifi
            .networks
            .iter()
            .find(|n| n.ssid.as_deref() == Some(ssid))?;
        network.auto_join
    }

    pub fn is_pending(&self, id: &str) -> bool {
        self.pending.contains(id)
    }

    fn row_count(&self) -> usize {
        match self.pane() {
            Pane::Wifi => match &self.snapshot.wifi {
                Some(wifi) if wifi.power => self.wifi_rows().len(),
                _ => 0,
            },
            Pane::Wired(i) if self.has_settings(&self.snapshot.interfaces[i]) => 2,
            Pane::Wired(_) | Pane::Empty => 0,
            Pane::Vpn => self.snapshot.vpns.len(),
        }
    }

    // --- helpers ---

    fn send(&self, cmd: Cmd) {
        // The backend only goes away when the app is quitting.
        let _ = self.cmds.send(cmd);
    }

    fn toast_ok(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::ok(message));
    }

    fn toast_err(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::error(message));
    }

    fn clamp(&mut self) {
        self.sel = self.sel.min(self.snapshot.interfaces.len());
        self.row = self.row.min(self.row_count().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: isize) {
        self.move_in(self.focus, delta);
    }

    fn move_in(&mut self, focus: Focus, delta: isize) {
        if focus == Focus::Sidebar {
            let last = self.snapshot.interfaces.len();
            self.sel = self.sel.saturating_add_signed(delta).min(last);
            self.row = 0;
        } else {
            let last = self.row_count().saturating_sub(1);
            self.row = self.row.saturating_add_signed(delta).min(last);
        }
    }

    // --- main screen keys ---

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Flow::Quit,
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            KeyCode::Char('s') => self.start_speedtest(),
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Pane,
                    Focus::Pane => Focus::Sidebar,
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Sidebar,
            KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Pane,
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Enter if self.focus == Focus::Sidebar => self.focus = Focus::Pane,
            _ if self.focus == Focus::Pane => match self.pane() {
                Pane::Wifi => self.wifi_key(key),
                Pane::Wired(i) => self.wired_key(key, i),
                Pane::Vpn => self.vpn_key(key),
                Pane::Empty => {}
            },
            _ => {}
        }
        Flow::Continue
    }

    fn wifi_key(&mut self, key: KeyEvent) {
        let Some(wifi) = &self.snapshot.wifi else {
            return;
        };
        let (power, names_hidden) = (wifi.power, wifi.names_hidden);
        if key.code == KeyCode::Char('p') {
            self.pending.insert("wifi".to_string());
            self.send(Cmd::SetWifiPower(!power));
            return;
        }
        if !power {
            return;
        }
        match key.code {
            KeyCode::Char('r') => {
                self.pending.insert("wifi".to_string());
                self.send(Cmd::Rescan);
            }
            KeyCode::Char('a') => {
                self.show_away = !self.show_away;
                self.clamp();
            }
            KeyCode::Char('L') if names_hidden => {
                self.pending.insert("wifi".to_string());
                self.send(Cmd::RequestLocation);
            }
            KeyCode::Enter => self.join_selected(),
            KeyCode::Char('i') => self.open_details(),
            _ => {}
        }
    }

    fn selected_wifi_row(&self) -> Option<WifiRow> {
        self.wifi_rows().get(self.row).copied()
    }

    fn selected_network(&self) -> Option<Network> {
        match self.selected_wifi_row()? {
            WifiRow::Net(i) => self.snapshot.wifi.as_ref()?.networks.get(i).cloned(),
            WifiRow::Away(_) => None,
        }
    }

    fn join_selected(&mut self) {
        let Some(wifi) = &self.snapshot.wifi else {
            return;
        };
        if let Some(WifiRow::Away(i)) = self.selected_wifi_row() {
            let name = wifi.saved_elsewhere[i].clone();
            return self.toast_err(format!("{name} is out of range."));
        }
        let names_hidden = wifi.names_hidden;
        let Some(network) = self.selected_network() else {
            return;
        };
        if network.connected {
            return self.toast_ok("Already connected to this network.");
        }
        if names_hidden {
            return self.toast_err("Network names are hidden. Press L to allow Location first.");
        }
        if network.saved || !network.security.needs_password() {
            self.pending.insert(network.id.clone());
            return self.send(Cmd::Join {
                network: network.id,
                password: None,
            });
        }
        let title = match &network.ssid {
            Some(ssid) => format!("Join {ssid}"),
            None => "Join hidden network".to_string(),
        };
        self.dialog = Some(Dialog::Join(Join {
            network: network.id,
            title,
            input: TextInput::masked(),
        }));
    }

    fn open_details(&mut self) {
        let Some(network) = self.selected_network() else {
            return;
        };
        self.details = None;
        if network.connected {
            self.send(Cmd::Details);
        }
        self.dialog = Some(Dialog::Details(DetailsDialog {
            network,
            password: None,
            confirm_forget: false,
            open_qr: false,
        }));
    }

    fn wired_key(&mut self, key: KeyEvent, index: usize) {
        let interface = &self.snapshot.interfaces[index];
        if key.code != KeyCode::Enter || !self.has_settings(interface) {
            return;
        }
        match self.row {
            0 => self.dialog = Some(Dialog::Ipv4(Ipv4Form::new(interface))),
            _ => self.dialog = Some(Dialog::Dns(DnsForm::new(interface))),
        }
    }

    fn vpn_key(&mut self, key: KeyEvent) {
        let Some(vpn) = self.snapshot.vpns.get(self.row).cloned() else {
            return;
        };
        match key.code {
            KeyCode::Enter if vpn.controllable => {
                self.pending.insert(vpn.id.clone());
                self.send(Cmd::SetVpn {
                    vpn: vpn.id,
                    on: !vpn.connected,
                });
            }
            KeyCode::Enter => {
                let reason = vpn
                    .detail
                    .unwrap_or_else(|| "It can't be controlled here.".into());
                self.toast_err(format!("{} is read-only: {reason}", vpn.name));
            }
            KeyCode::Char('i') => match vpn.detail {
                Some(detail) => self.toast_ok(format!("{}: {detail}", vpn.name)),
                None => self.toast_ok(format!("{}: no more details.", vpn.name)),
            },
            _ => {}
        }
    }

    // --- dialogs ---

    fn dialog_key(&mut self, key: KeyEvent) -> Flow {
        let Some(mut dialog) = self.dialog.take() else {
            return Flow::Continue;
        };
        let outcome = match &mut dialog {
            Dialog::Join(d) => self.join_key(d, key),
            Dialog::Details(d) => self.details_key(d, key),
            Dialog::Ipv4(d) => self.ipv4_key(d, key),
            Dialog::Dns(d) => self.dns_key(d, key),
            Dialog::Help => close_on_esc(key),
        };
        match outcome {
            Outcome::Stay => self.dialog = Some(dialog),
            Outcome::Close => {}
        }
        Flow::Continue
    }

    fn join_key(&mut self, d: &mut Join, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Tab => d.input.masked = !d.input.masked,
            KeyCode::Enter if !d.input.value.is_empty() => {
                self.pending.insert(d.network.clone());
                self.send(Cmd::Join {
                    network: d.network.clone(),
                    password: Some(d.input.value.clone()),
                });
                return Outcome::Close;
            }
            _ => {
                d.input.handle(key);
            }
        }
        Outcome::Stay
    }

    fn details_key(&mut self, d: &mut DetailsDialog, key: KeyEvent) -> Outcome {
        let Some(ssid) = d.network.ssid.clone() else {
            return close_on_esc(key);
        };
        if d.confirm_forget {
            return match key.code {
                KeyCode::Char('y') => {
                    self.pending.insert(ssid.clone());
                    self.send(Cmd::Forget { ssid });
                    Outcome::Close
                }
                _ => {
                    d.confirm_forget = false;
                    Outcome::Stay
                }
            };
        }
        let saved = d.network.saved;
        let open = d.network.security == Security::Open;
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Char('c') => self.copy_ip(d),
            KeyCode::Char('y') if saved => {
                if d.password.is_some() {
                    d.password = None;
                } else {
                    self.send(Cmd::RevealPassword { ssid });
                }
            }
            KeyCode::Char('y') if open => d.open_qr = !d.open_qr,
            KeyCode::Char('a') if saved => {
                let on = !self.auto_join(&ssid).unwrap_or(true);
                self.pending.insert(ssid.clone());
                self.send(Cmd::SetAutoJoin { ssid, on });
            }
            KeyCode::Char('d') if saved => d.confirm_forget = true,
            _ => {}
        }
        Outcome::Stay
    }

    fn copy_ip(&mut self, d: &DetailsDialog) {
        let ip = d
            .network
            .connected
            .then(|| self.wifi_ipv4().map(|v| v.address.clone()))
            .flatten();
        match ip {
            Some(ip) => {
                telmo_kit::os::copy(&ip);
                self.toast_ok(format!("Copied {ip}"));
            }
            None => self.toast_err("Only the connected network has an IP address."),
        }
    }

    fn ipv4_key(&mut self, d: &mut Ipv4Form, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Tab => d.focus = if d.manual { (d.focus + 1) % 4 } else { 0 },
            KeyCode::Left if d.focus == 0 => d.manual = false,
            KeyCode::Right if d.focus == 0 => d.manual = true,
            KeyCode::Char(' ') if d.focus == 0 => d.manual = !d.manual,
            KeyCode::Enter => match d.config() {
                Ok(config) => {
                    self.save_config(&d.interface, config);
                    return Outcome::Close;
                }
                Err(message) => d.error = Some(message),
            },
            _ if d.manual => {
                let field = match d.focus {
                    1 => Some(&mut d.address),
                    2 => Some(&mut d.subnet),
                    3 => Some(&mut d.router),
                    _ => None,
                };
                if let Some(field) = field {
                    field.handle(key);
                    d.error = None;
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    fn dns_key(&mut self, d: &mut DnsForm, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => match d.config() {
                Ok(config) => {
                    self.save_config(&d.interface, config);
                    return Outcome::Close;
                }
                Err(message) => d.error = Some(message),
            },
            _ => {
                d.input.handle(key);
                d.error = None;
            }
        }
        Outcome::Stay
    }

    fn save_config(&mut self, interface: &str, config: Ipv4Config) {
        self.pending.insert(interface.to_string());
        self.send(Cmd::SetIpv4 {
            interface: interface.to_string(),
            config,
        });
    }

    // --- speedtest ---

    fn start_speedtest(&mut self) {
        self.screen = Screen::Speed;
        if self.speed.running {
            return;
        }
        self.speed = Speed {
            running: true,
            last: telmo_speed::history().into_iter().next(),
            ..Speed::default()
        };
        let tx = self.events.clone();
        if self.mock {
            mock::fake_speedtest(tx);
        } else {
            tokio::spawn(telmo_speed::run(move |update| {
                let _ = tx.send(Event::Speed(update));
            }));
        }
    }

    fn speed_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Esc => self.screen = Screen::Main,
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            KeyCode::Enter => self.start_speedtest(),
            KeyCode::Char('c') if self.speed.finished() => {
                let summary = self.speed.summary();
                telmo_kit::os::copy(&summary);
                self.toast_ok(format!("Copied: {summary}"));
            }
            KeyCode::Char('c') => self.toast_err("The test hasn't finished yet."),
            _ => {}
        }
        Flow::Continue
    }

    fn speed_update(&mut self, update: Update) {
        let finished = matches!(update, Update::Finished);
        if let Update::Failed(message) = &update {
            self.toast_err(message.clone());
        }
        self.speed.apply(update);
        if finished {
            telmo_speed::remember(self.record());
        }
    }

    fn record(&self) -> Record {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Record {
            at,
            network: self.primary_summary(),
            down_mbps: self.speed.down.mbps,
            up_mbps: self.speed.up.mbps,
            latency_ms: self.speed.idle_ms.unwrap_or(0.0),
        }
    }

    /// The primary interface as the user knows it: the network name on Wi-Fi.
    pub fn primary_summary(&self) -> Option<String> {
        let primary = self.snapshot.primary.as_ref()?;
        let interface = self
            .snapshot
            .interfaces
            .iter()
            .find(|i| &i.name == primary)?;
        match interface.kind {
            InterfaceKind::Wifi => Some(interface.summary.clone()),
            _ => Some(interface.name.clone()),
        }
    }
}

enum Outcome {
    Stay,
    Close,
}

fn close_on_esc(key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
        _ => Outcome::Stay,
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if self.dialog.is_some() {
            return self.dialog_key(key);
        }
        match self.screen {
            Screen::Main => self.main_key(key),
            Screen::Speed => self.speed_key(key),
        }
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        let under = self.hits.at(event.column, event.row);
        match (event.kind, under) {
            (MouseEventKind::Down(MouseButton::Left), Some(click)) => self.click(click),
            (MouseEventKind::ScrollUp, Some(click)) => self.scroll(click, -1),
            (MouseEventKind::ScrollDown, Some(click)) => self.scroll(click, 1),
            _ => Flow::Continue,
        }
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Snapshot(snapshot) => {
                self.snapshot = snapshot;
                self.clamp();
            }
            Event::Done { target, result } => {
                self.pending.remove(&target);
                match result {
                    Ok(message) => self.toast_ok(message),
                    Err(message) => self.toast_err(message),
                }
            }
            Event::Details(details) => self.details = Some(details),
            Event::Password { ssid, password } => self.password_arrived(&ssid, password),
            Event::Speed(update) => self.speed_update(update),
        }
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        self.tick += 1;
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        let scanning = self.snapshot.wifi.as_ref().is_some_and(|w| w.scanning);
        !self.pending.is_empty() || scanning || self.speed.running || self.toast.is_some()
    }
}

impl App {
    /// A click does what the matching key does.
    fn click(&mut self, click: Click) -> Flow {
        match click {
            Click::Key(code) | Click::DialogKey(code) => self.press(code),
            Click::Outside => self.press(KeyCode::Esc),
            Click::Inside => Flow::Continue,
            Click::Pane(focus) => {
                self.focus = focus;
                Flow::Continue
            }
            Click::Row(Focus::Pane, row) if self.focus == Focus::Pane && self.row == row => {
                self.press(KeyCode::Enter)
            }
            Click::Row(Focus::Sidebar, sel) if self.focus == Focus::Sidebar && self.sel == sel => {
                self.press(KeyCode::Enter)
            }
            Click::Row(focus, index) => {
                self.focus = focus;
                if focus == Focus::Sidebar {
                    self.sel = index;
                    self.row = 0;
                } else {
                    self.row = index;
                }
                Flow::Continue
            }
        }
    }

    fn scroll(&mut self, click: Click, delta: isize) -> Flow {
        if let Click::Row(focus, _) | Click::Pane(focus) = click {
            self.focus = focus;
            self.move_in(focus, delta);
        }
        Flow::Continue
    }

    fn press(&mut self, code: KeyCode) -> Flow {
        telmo_kit::App::key(self, KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn password_arrived(&mut self, ssid: &str, password: Result<String, String>) {
        let password = match password {
            Ok(password) => password,
            Err(message) => {
                return self.toast_err(message);
            }
        };
        let Some(Dialog::Details(d)) = &mut self.dialog else {
            return;
        };
        if d.network.ssid.as_deref() != Some(ssid) {
            return;
        }
        d.password = Some(password);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if !self.mock {
            telmo_kit::cache::save("net", &self.snapshot);
        }
    }
}

impl Ipv4Form {
    fn new(interface: &Interface) -> Self {
        let manual = interface.config.as_ref().and_then(|c| c.manual.clone());
        let current = interface.ipv4.clone().unwrap_or_default();
        let shown = manual.clone().unwrap_or(ManualIpv4 {
            address: current.address.clone(),
            subnet: prefix_to_mask(current.prefix),
            router: current.router.clone().unwrap_or_default(),
        });
        Self {
            interface: interface.id.clone(),
            name: interface.name.clone(),
            manual: manual.is_some(),
            focus: 0,
            address: TextInput::with_value(shown.address),
            subnet: TextInput::with_value(shown.subnet),
            router: TextInput::with_value(shown.router),
            dns: interface
                .config
                .as_ref()
                .map(|c| c.dns.clone())
                .unwrap_or_default(),
            error: None,
        }
    }

    fn config(&self) -> Result<Ipv4Config, String> {
        let manual = if self.manual {
            Some(ManualIpv4 {
                address: valid_ipv4("address", &self.address.value)?,
                subnet: valid_ipv4("subnet", &self.subnet.value)?,
                router: valid_ipv4("router", &self.router.value)?,
            })
        } else {
            None
        };
        Ok(Ipv4Config {
            manual,
            dns: self.dns.clone(),
        })
    }
}

impl DnsForm {
    fn new(interface: &Interface) -> Self {
        let config = interface.config.clone();
        let dns = config
            .as_ref()
            .map(|c| c.dns.join(", "))
            .unwrap_or_default();
        Self {
            interface: interface.id.clone(),
            name: interface.name.clone(),
            input: TextInput::with_value(dns),
            manual: config.and_then(|c| c.manual),
            error: None,
        }
    }

    fn config(&self) -> Result<Ipv4Config, String> {
        let mut dns = Vec::new();
        for server in self
            .input
            .value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if server.parse::<IpAddr>().is_err() {
                return Err(format!("\"{server}\" isn't an IP address."));
            }
            dns.push(server.to_string());
        }
        Ok(Ipv4Config {
            manual: self.manual.clone(),
            dns,
        })
    }
}

fn valid_ipv4(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    match value.parse::<Ipv4Addr>() {
        Ok(_) => Ok(value.to_string()),
        Err(_) => Err(format!("The {field} must look like 192.168.1.10.")),
    }
}

fn prefix_to_mask(prefix: u8) -> String {
    let bits = u32::MAX
        .checked_shl(32 - u32::from(prefix.min(32)))
        .unwrap_or(0);
    Ipv4Addr::from(bits).to_string()
}

/// Escape the characters that are special in a Wi-Fi QR payload.
fn escape_wifi_field(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn wifi_payload(security: Security, ssid: &str, password: &str) -> String {
    let ssid = escape_wifi_field(ssid);
    match security {
        Security::Open => format!("WIFI:T:nopass;S:{ssid};;"),
        Security::Wep => format!("WIFI:T:WEP;S:{ssid};P:{};;", escape_wifi_field(password)),
        _ => format!("WIFI:T:WPA;S:{ssid};P:{};;", escape_wifi_field(password)),
    }
}

impl Qr {
    fn new(network: &Network, password: &str) -> Self {
        let ssid = network.ssid.as_deref().unwrap_or_default();
        Self {
            payload: wifi_payload(network.security, ssid, password),
        }
    }

    /// The code as rows of text, two modules per row using half blocks.
    /// Light modules are drawn, so it scans on a dark terminal.
    pub fn lines(&self) -> Option<Vec<String>> {
        let code = QrCode::new(self.payload.as_bytes()).ok()?;
        let width = code.width();
        let dark = |x: isize, y: isize| {
            let inside = (0..width as isize).contains(&x) && (0..width as isize).contains(&y);
            inside && code[(x as usize, y as usize)] == qrcode::Color::Dark
        };
        let mut lines = Vec::new();
        let mut y = -1;
        while y <= width as isize {
            let line = (-1..=width as isize)
                .map(|x| match (dark(x, y), dark(x, y + 1)) {
                    (false, false) => '█',
                    (false, true) => '▀',
                    (true, false) => '▄',
                    (true, true) => ' ',
                })
                .collect();
            lines.push(line);
            y += 2;
        }
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_payload_escapes_special_characters() {
        assert_eq!(
            wifi_payload(Security::Personal, r#"a;b,c:d"e\f"#, r#"p;w:"\,"#),
            r#"WIFI:T:WPA;S:a\;b\,c\:d\"e\\f;P:p\;w\:\"\\\,;;"#
        );
    }

    #[test]
    fn open_network_payload_has_no_password() {
        assert_eq!(
            wifi_payload(Security::Open, "Cafe", ""),
            "WIFI:T:nopass;S:Cafe;;"
        );
    }
}
