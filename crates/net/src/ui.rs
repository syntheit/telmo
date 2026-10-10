//! Drawing for telmo-net: a pure function of the app state.

use crate::app::{
    App, Click, DetailsDialog, Dialog, DnsForm, Focus, Ipv4Form, Join, Meter, Pane, Screen, WifiRow,
};
use crate::model::*;
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span},
};
use telmo_kit::{
    input::TextInput,
    theme,
    widgets::{self, fit},
};
use telmo_speed::Phase;

pub fn draw(app: &App, frame: &mut Frame) {
    app.hits.clear();
    let screen = widgets::screen(frame.area());
    match app.screen {
        Screen::Main => {
            widgets::header(frame, screen.header, "Network", main_status(app));
            draw_main(app, frame, screen.body);
        }
        Screen::Speed => {
            widgets::header(
                frame,
                screen.header,
                "Network › Speedtest",
                speed_status(app),
            );
            draw_speed(app, frame, screen.body);
        }
    }
    if let Some(toast) = app.toast.as_ref().filter(|t| !t.expired()) {
        toast.render(frame, screen.toast);
    }
    let bar = key_bar(app);
    let areas = widgets::keys(frame, screen.keys, &bar);
    for ((key, _), area) in bar.iter().zip(areas) {
        if let Some(code) = key_code(key) {
            app.hits.add(area, Click::Key(code));
        }
    }
    if let Some(dialog) = &app.dialog {
        draw_dialog(app, frame, dialog);
    }
}

fn main_status(app: &App) -> Line<'static> {
    match &app.snapshot.primary {
        Some(primary) => Line::from(vec![
            Span::styled("● ", theme::ok()),
            Span::styled("online", theme::text()),
            Span::styled(format!("  via {primary}"), theme::dim()),
        ]),
        None => Line::from(vec![
            Span::styled("○ ", theme::dim()),
            Span::styled("offline", theme::dim()),
        ]),
    }
}

fn speed_status(app: &App) -> Line<'static> {
    let via = app
        .primary_summary()
        .map(|name| format!(" · via {name}"))
        .unwrap_or_default();
    Line::styled(format!("Cloudflare{via}"), theme::dim())
}

fn dot(on: bool) -> Span<'static> {
    if on {
        Span::styled("●", theme::ok())
    } else {
        Span::styled("○", theme::dim())
    }
}

// --- key bar and help ---

/// The key a label in a key bar or hint stands for.
fn key_code(label: &str) -> Option<KeyCode> {
    let mut chars = label.chars();
    match (label, chars.next(), chars.next()) {
        ("↵", ..) => Some(KeyCode::Enter),
        ("esc", ..) => Some(KeyCode::Esc),
        ("tab", ..) => Some(KeyCode::Tab),
        (_, Some(c), None) => Some(KeyCode::Char(c)),
        _ => None,
    }
}

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut keys: Vec<(&str, &str)> = match app.screen {
        Screen::Speed => return vec![("↵", "run again"), ("c", "copy result"), ("esc", "back")],
        Screen::Main if app.focus == Focus::Sidebar => {
            vec![("↵", "open"), ("s", "speedtest")]
        }
        Screen::Main => match app.pane() {
            Pane::Wifi if wifi_off(app) => vec![("p", "turn on"), ("s", "speedtest")],
            Pane::Wifi => vec![
                ("↵", "join"),
                ("i", "details"),
                ("r", "rescan"),
                ("s", "speedtest"),
                ("p", "power"),
            ],
            Pane::Wired(i) if app.has_settings(&app.snapshot.interfaces[i]) => {
                vec![("↵", "edit")]
            }
            Pane::Vpn => vec![("↵", "connect/disconnect"), ("i", "details")],
            Pane::Wired(_) | Pane::Empty => vec![("s", "speedtest")],
        },
    };
    keys.extend([("?", "more"), ("esc", "close")]);
    keys
}

fn wifi_off(app: &App) -> bool {
    app.snapshot.wifi.as_ref().is_some_and(|w| !w.power)
}

fn help_lines(app: &App) -> Vec<(&'static str, &'static str)> {
    if app.screen == Screen::Speed {
        return vec![
            ("↵", "run the test again"),
            ("c", "copy the result"),
            ("esc", "back to the network list"),
            ("q", "quit"),
        ];
    }
    let mut lines = vec![
        ("j k ↑ ↓", "move"),
        ("tab", "switch between interfaces and the pane"),
        ("h l ← →", "interfaces / pane"),
        ("s", "speedtest"),
        ("click", "select, click again to act, wheel scrolls"),
    ];
    match app.pane() {
        Pane::Wifi => lines.extend([
            ("↵", "join the selected network"),
            ("i", "details: password, QR, auto-join, forget"),
            ("a", "show or hide saved networks out of range"),
            ("p", "turn Wi-Fi on or off"),
            ("r", "rescan"),
            ("L", "allow Location (names hidden)"),
        ]),
        Pane::Wired(_) => lines.push(("↵", "edit IPv4 or DNS")),
        Pane::Vpn => lines.extend([("↵", "connect or disconnect"), ("i", "details")]),
        Pane::Empty => {}
    }
    lines.extend([("?", "this help"), ("esc q", "close dialog, then quit")]);
    lines
}

// --- main screen ---

fn draw_main(app: &App, frame: &mut Frame, body: Rect) {
    let [sidebar, pane] =
        Layout::horizontal([Constraint::Length(26), Constraint::Fill(1)]).areas(body);
    draw_sidebar(app, frame, sidebar);
    match app.pane() {
        Pane::Wifi => draw_wifi(app, frame, pane),
        Pane::Wired(i) => draw_wired(app, frame, pane, &app.snapshot.interfaces[i]),
        Pane::Vpn => draw_vpn(app, frame, pane),
        Pane::Empty => {}
    }
}

fn draw_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::pane("Interfaces", app.focus == Focus::Sidebar, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Focus::Sidebar));
    let mut lines = Vec::new();
    for interface in &app.snapshot.interfaces {
        lines.push(entry_line(&interface.name, interface.connected));
        lines.push(summary_line(&interface.summary));
        lines.push(Line::raw(""));
    }
    let vpns = &app.snapshot.vpns;
    let connected: Vec<&str> = vpns
        .iter()
        .filter(|v| v.connected)
        .map(|v| v.name.as_str())
        .collect();
    lines.push(entry_line("VPN", !connected.is_empty()));
    let summary = if connected.is_empty() {
        "off".to_string()
    } else {
        connected.join(", ")
    };
    lines.push(summary_line(&summary));
    let drawn = widgets::rows(frame, inner, lines, Some(app.sel * 3));
    for (line, rect) in drawn {
        app.hits.add(rect, Click::Row(Focus::Sidebar, line / 3));
    }
}

fn entry_line(name: &str, on: bool) -> Line<'static> {
    Line::from(vec![Span::raw(fit(name, 18)), dot(on)])
}

fn summary_line(summary: &str) -> Line<'static> {
    Line::styled(fit(summary, 20), theme::dim())
}

fn pane_label(dot_on: bool, text: String) -> Line<'static> {
    Line::from(vec![
        dot(dot_on),
        Span::styled(format!(" {text}"), theme::dim()),
    ])
}

fn draw_wifi(app: &App, frame: &mut Frame, area: Rect) {
    let active = app.focus == Focus::Pane;
    let Some(wifi) = &app.snapshot.wifi else {
        return;
    };
    let right = if app.is_pending("wifi") || wifi.scanning {
        let word = if wifi.scanning { "scanning" } else { "working" };
        Line::styled(
            format!("{} {word}", widgets::spinner(app.tick)),
            theme::info(),
        )
    } else {
        pane_label(
            wifi.power,
            if wifi.power { "on" } else { "off" }.to_string(),
        )
    };
    if wifi.names_hidden {
        return draw_wifi_hidden(app, frame, area);
    }
    let block = widgets::pane("Wi-Fi", active, None, Some(right));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Focus::Pane));
    if !wifi.power {
        let lines = vec![
            Line::raw(""),
            Line::styled("   Wi-Fi is off", theme::dim()),
            Line::raw(""),
            key_line("p", "turn Wi-Fi on"),
        ];
        return widgets::text(frame, inner, lines);
    }
    draw_wifi_rows(app, frame, inner, vec![]);
}

/// macOS hides the names until Location is allowed: a yellow warning on top.
fn draw_wifi_hidden(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::warning_pane("Wi-Fi names hidden");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Focus::Pane));
    let intro = vec![
        Line::styled("macOS hides network names until Location is", theme::text()),
        Line::styled("allowed for Telmo.", theme::text()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("L", theme::accent()),
            Span::styled(" allow Location", theme::dim()),
        ]),
        Line::raw(""),
    ];
    draw_wifi_rows(app, frame, inner, intro);
}

fn key_line(key: &str, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {key}"), theme::accent()),
        Span::styled(format!(" {label}"), theme::dim()),
    ])
}

/// Network rows after `intro` lines, plus the "saved networks" hint.
fn draw_wifi_rows(app: &App, frame: &mut Frame, area: Rect, intro: Vec<Line<'static>>) {
    let Some(wifi) = &app.snapshot.wifi else {
        return;
    };
    let offset = intro.len();
    let mut lines = intro;
    for row in app.wifi_rows() {
        lines.push(match row {
            WifiRow::Net(i) => network_line(app, wifi, &wifi.networks[i]),
            WifiRow::Away(i) => Line::from(vec![
                Span::styled(fit(&wifi.saved_elsewhere[i], 30), theme::dim()),
                Span::raw("       "),
                Span::styled("saved", theme::dim()),
            ]),
        });
    }
    let away = wifi.saved_elsewhere.len();
    if away > 0 {
        let verb = if app.show_away { "hide" } else { "show" };
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("a", theme::accent()),
            Span::styled(
                format!(" {verb} {away} saved networks out of range"),
                theme::dim(),
            ),
        ]));
    }
    let selected = (app.focus == Focus::Pane).then_some(offset + app.row);
    let drawn = widgets::rows(frame, area, lines, selected);
    add_row_hits(app, &drawn, offset, app.wifi_rows().len());
}

/// Rows of the pane start at line `first` and there are `count` of them.
fn add_row_hits(app: &App, drawn: &[(usize, Rect)], first: usize, count: usize) {
    for &(line, rect) in drawn {
        if (first..first + count).contains(&line) {
            app.hits.add(rect, Click::Row(Focus::Pane, line - first));
        }
    }
}

fn network_line(app: &App, wifi: &Wifi, network: &Network) -> Line<'static> {
    let name = match (&network.ssid, wifi.names_hidden) {
        (Some(ssid), _) => ssid.clone(),
        (None, true) => "‹name hidden›".to_string(),
        (None, false) => "hidden network".to_string(),
    };
    let tag = if app.is_pending(&network.id) {
        Span::styled(
            format!("{} joining…", widgets::spinner(app.tick)),
            theme::info(),
        )
    } else if network.connected {
        Span::styled("● connected", theme::ok())
    } else if network.saved {
        Span::styled("saved", theme::dim())
    } else if network.security == Security::Open {
        Span::styled("open", theme::dim())
    } else {
        Span::raw("")
    };
    let mut spans = vec![Span::raw(fit(&name, 30))];
    spans.extend(widgets::signal(network.strength));
    spans.push(Span::raw("   "));
    spans.push(tag);
    Line::from(spans)
}

fn speed_label(mbps: u32) -> String {
    if mbps >= 1000 {
        format!("{} Gbps", mbps / 1000)
    } else {
        format!("{mbps} Mbps")
    }
}

fn draw_wired(app: &App, frame: &mut Frame, area: Rect, interface: &Interface) {
    let right = if interface.connected {
        let link = interface
            .link_mbps
            .map(|m| format!(" · {}", speed_label(m)));
        pane_label(true, format!("connected{}", link.unwrap_or_default()))
    } else {
        pane_label(false, "not connected".to_string())
    };
    let block = widgets::pane(&interface.name, app.focus == Focus::Pane, None, Some(right));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Focus::Pane));

    let mut lines = vec![Line::raw("")];
    match &interface.ipv4 {
        Some(ipv4) => {
            lines.push(info_line("ip", &ipv4.address));
            lines.push(info_line(
                "router",
                ipv4.router.as_deref().unwrap_or("none"),
            ));
            lines.push(info_line("dns", &ipv4.dns.join(", ")));
        }
        None => lines.push(Line::styled("not connected", theme::dim())),
    }
    let mut selected = None;
    let mut settings_start = None;
    if app.has_settings(interface) {
        let rule = "─".repeat((inner.width as usize).saturating_sub(16));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("settings ", theme::dim()),
            Span::styled(rule, theme::faint()),
        ]));
        settings_start = Some(lines.len());
        if app.focus == Focus::Pane {
            selected = Some(lines.len() + app.row);
        }
        lines.extend(setting_lines(app, interface));
    }
    let drawn = widgets::rows(frame, inner, lines, selected);
    if let Some(first) = settings_start {
        add_row_hits(app, &drawn, first, 2);
    }
}

fn info_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(fit(label, 10), theme::dim()),
        Span::raw(value.to_string()),
    ])
}

fn setting_lines(app: &App, interface: &Interface) -> Vec<Line<'static>> {
    let config = interface.config.clone().unwrap_or(Ipv4Config {
        manual: None,
        dns: vec![],
    });
    let saving = app.is_pending(&interface.id);
    let pending = format!("{} saving…", widgets::spinner(app.tick));
    let ipv4 = match &config.manual {
        _ if saving => pending.clone(),
        Some(m) => format!("Manual · {}", m.address),
        None => "Automatic (DHCP)".to_string(),
    };
    let dns = match config.dns.is_empty() {
        _ if saving => pending,
        true => "Automatic".to_string(),
        false => config.dns.join(", "),
    };
    [("IPv4", ipv4), ("DNS", dns)]
        .into_iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::raw(fit(label, 14)),
                Span::raw(fit(&value, 34)),
                Span::styled("›", theme::dim()),
            ])
        })
        .collect()
}

fn draw_vpn(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::pane("VPN", app.focus == Focus::Pane, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.hits.add(area, Click::Pane(Focus::Pane));
    if app.snapshot.vpns.is_empty() {
        let lines = vec![
            Line::raw(""),
            Line::styled("   No VPNs set up on this system.", theme::dim()),
        ];
        return widgets::text(frame, inner, lines);
    }
    let mut lines = vec![Line::raw("")];
    for vpn in &app.snapshot.vpns {
        let tag = if app.is_pending(&vpn.id) {
            Span::styled(
                format!("{} working…", widgets::spinner(app.tick)),
                theme::info(),
            )
        } else if vpn.connected {
            Span::styled("● connected", theme::ok())
        } else {
            Span::styled("○ off", theme::dim())
        };
        lines.push(Line::from(vec![Span::raw(fit(&vpn.name, 30)), tag]));
    }
    let selected = (app.focus == Focus::Pane).then_some(1 + app.row);
    let drawn = widgets::rows(frame, inner, lines, selected);
    add_row_hits(app, &drawn, 1, app.snapshot.vpns.len());
}

// --- speedtest ---

fn draw_speed(app: &App, frame: &mut Frame, body: Rect) {
    let [meters, latency, _, last, _] = Layout::vertical([
        Constraint::Length(10),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(body);
    let [down, up] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(meters);
    let speed = &app.speed;
    draw_meter(
        app,
        frame,
        down,
        "Download",
        &speed.down,
        Phase::Download,
        theme::CYAN,
    );
    draw_meter(
        app,
        frame,
        up,
        "Upload",
        &speed.up,
        Phase::Upload,
        theme::MAGENTA,
    );

    let block = widgets::pane("Latency", false, None, None);
    let inner = block.inner(latency);
    frame.render_widget(block, latency);
    let ms = |value: Option<f64>, decimals: usize| {
        value.map_or("--".to_string(), |v| format!("{v:.decimals$} ms"))
    };
    let line = Line::from(vec![
        Span::styled("  idle ", theme::dim()),
        Span::raw(format!("{}    ", ms(speed.idle_ms, 0))),
        Span::styled("jitter ", theme::dim()),
        Span::raw(format!("{}    ", ms(speed.jitter_ms, 1))),
        Span::styled("under load ", theme::dim()),
        Span::raw(ms(speed.loaded_ms, 0)),
    ]);
    widgets::text(frame, inner, vec![line]);
    frame.render_widget(last_run_line(app), last);
}

fn draw_meter(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    title: &str,
    meter: &Meter,
    phase: Phase,
    color: ratatui::style::Color,
) {
    let measuring = app.speed.phase == Some(phase);
    let block = widgets::pane(title, measuring, None, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = inner.width as usize;
    let style = Style::new().fg(color);

    let mut lines = vec![Line::raw("")];
    let mut digits = widgets::big_digits(&format!("{:.1}", meter.mbps), style);
    digits[4].spans.push(Span::styled("  Mbps", theme::dim()));
    for mut row in digits {
        row.spans.insert(0, Span::raw("   "));
        lines.push(row);
    }
    lines.push(Line::raw(""));

    let status = if meter.done {
        Span::styled("done", theme::ok())
    } else if measuring {
        Span::styled(
            format!("{} measuring", widgets::spinner(app.tick)),
            theme::info(),
        )
    } else {
        Span::styled("waiting", theme::dim())
    };
    let mut gauge = vec![Span::raw("   ")];
    gauge.extend(widgets::gauge(
        meter.fraction,
        width.saturating_sub(18),
        style,
    ));
    gauge.push(Span::raw("  "));
    gauge.push(status);
    lines.push(Line::from(gauge));
    widgets::text(frame, inner, lines);
}

fn last_run_line(app: &App) -> Line<'static> {
    let Some(record) = &app.speed.last else {
        return Line::from(vec![
            Span::styled(" last run  ", theme::dim()),
            Span::styled("none yet", theme::dim()),
        ]);
    };
    Line::from(vec![
        Span::styled(" last run  ", theme::dim()),
        Span::raw(format!("{}   ", ago(record.at))),
        Span::styled(format!("↓ {:.1}", record.down_mbps), theme::info()),
        Span::raw("   "),
        Span::styled(
            format!("↑ {:.1}", record.up_mbps),
            Style::new().fg(theme::MAGENTA),
        ),
        Span::raw(format!("   {:.0} ms", record.latency_ms)),
    ])
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
fn ago(at: u64) -> String {
    let secs = telmo_kit::time::unix_now().saturating_sub(at);
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

// --- dialogs ---

/// `widgets::dialog` plus the click areas: a click inside does nothing, a
/// click anywhere else closes it.
fn open_dialog(app: &App, frame: &mut Frame, title: &str, width: u16, height: u16) -> Rect {
    let inner = widgets::dialog(frame, title, width, height);
    app.hits.add(frame.area(), Click::Outside);
    app.hits.add(inner.outer(Margin::new(1, 1)), Click::Inside);
    inner
}

/// A hint line at `row` inside the dialog whose items can be clicked.
fn dialog_hint(app: &App, inner: Rect, row: usize, bindings: &[(&str, &str)]) -> Line<'static> {
    let line = Rect {
        y: inner.y + row as u16,
        height: 1,
        ..inner
    };
    for ((key, _), area) in bindings.iter().zip(widgets::hint_areas(line, bindings)) {
        if let Some(code) = key_code(key) {
            app.hits.add(area, Click::DialogKey(code));
        }
    }
    widgets::hint(bindings)
}

fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    match dialog {
        Dialog::Join(d) => draw_join(app, frame, d),
        Dialog::Details(d) => draw_details(app, frame, d),
        Dialog::Ipv4(d) => draw_ipv4(app, frame, d),
        Dialog::Dns(d) => draw_dns(app, frame, d),
        Dialog::Help => draw_help(app, frame),
    }
}

fn field_line(label: &str, input: &TextInput, width: usize, focused: bool) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("  {}", fit(label, 10)), theme::dim())];
    spans.extend(input.spans(width, focused));
    Line::from(spans)
}

fn draw_join(app: &App, frame: &mut Frame, d: &Join) {
    let inner = open_dialog(app, frame, &d.title, 50, 7);
    let bindings = [("↵", "join"), ("tab", "show password"), ("esc", "cancel")];
    let hint = dialog_hint(app, inner, 3, &bindings);
    let lines = vec![
        Line::raw(""),
        field_line("password", &d.input, 32, true),
        Line::raw(""),
        hint,
    ];
    widgets::text(frame, inner, lines);
}

fn signal_summary(network: &Network) -> String {
    let strength = match network.strength {
        75.. => "strong",
        50.. => "good",
        25.. => "fair",
        _ => "weak",
    };
    let band = match network.band {
        Some(Band::G2) => " · 2.4 GHz",
        Some(Band::G5) => " · 5 GHz",
        Some(Band::G6) => " · 6 GHz",
        None => "",
    };
    let security = match network.security {
        Security::Open => "open",
        Security::Wep => "WEP",
        Security::Personal => "WPA2",
        Security::Wpa3Personal => "WPA3",
        Security::Enterprise => "802.1X",
    };
    format!("{strength}{band} · {security}")
}

fn draw_details(app: &App, frame: &mut Frame, d: &DetailsDialog) {
    let name = d
        .network
        .ssid
        .clone()
        .unwrap_or_else(|| "hidden network".to_string());
    let password = match (&d.password, d.network.saved) {
        (Some(password), _) => password.clone(),
        (None, true) => "••••••••••".to_string(),
        (None, false) if d.network.security == Security::Open => "none (open network)".to_string(),
        (None, false) => "not saved".to_string(),
    };
    let hint = details_hint(app, d);
    // The code is 16 rows or so: beside the numbers it needs a tall terminal.
    let code = d.qr().map(|qr| qr.lines());
    let code_rows = code.as_ref().map_or(0, |c| c.as_ref().map_or(1, Vec::len));
    let rows = if d.network.saved { 13 } else { 12 };
    if code.is_some() && rows + code_rows > frame.area().height as usize {
        return draw_share(app, frame, &name, &password, code.flatten(), &hint);
    }
    let inner = open_dialog(app, frame, &name, 70, (rows + code_rows) as u16);
    let ipv4 = d.network.connected.then(|| app.wifi_ipv4()).flatten();
    let none = "—".to_string();
    let public_ip = match (&app.details, d.network.connected) {
        (Some(details), _) => details.public_ip.clone().unwrap_or(none.clone()),
        (None, true) => "…".to_string(),
        (None, false) => none.clone(),
    };
    let rows = [
        ("ip", ipv4.map_or(none.clone(), |v| v.address.clone())),
        (
            "router",
            ipv4.and_then(|v| v.router.clone()).unwrap_or(none.clone()),
        ),
        ("dns", ipv4.map_or(none.clone(), |v| v.dns.join(", "))),
        ("public ip", public_ip),
        ("signal", signal_summary(&d.network)),
        ("password", password),
    ];
    let auto_join =
        d.network.saved.then(
            || match d.network.ssid.as_deref().and_then(|s| app.auto_join(s)) {
                Some(true) => "on",
                Some(false) => "off",
                None => "—",
            },
        );
    let mut lines = vec![Line::raw("")];
    lines.extend(rows.map(|(label, value)| {
        Line::from(vec![
            Span::styled(format!("   {}", fit(label, 11)), theme::dim()),
            Span::raw(value),
        ])
    }));
    if let Some(state) = auto_join {
        app.hits.add(
            Rect {
                y: inner.y + lines.len() as u16,
                height: 1,
                ..inner
            },
            Click::DialogKey(KeyCode::Char('a')),
        );
        lines.push(Line::from(vec![
            Span::styled(format!("   {}", fit("auto-join", 11)), theme::dim()),
            Span::raw(state),
        ]));
    }
    lines.push(Line::raw(""));
    if let Some(code) = code {
        lines.extend(qr_lines(code));
        lines.push(Line::raw(""));
    }
    let row = lines.len();
    lines.push(details_footer(app, inner, row, d, hint));
    widgets::text(frame, inner, lines);
}

/// What the footer of the details dialog says and offers.
fn details_hint(app: &App, d: &DetailsDialog) -> Vec<(&'static str, &'static str)> {
    let open = d.network.security == Security::Open;
    if d.network.saved {
        let reveal = match (d.password.is_some(), app.snapshot.caps.reveal_touch_id) {
            (true, _) => "hide password",
            (false, true) => "show password (Touch ID)",
            (false, false) => "show password",
        };
        let copy = if d.password.is_some() {
            "copy password"
        } else {
            "copy ip"
        };
        vec![
            ("y", reveal),
            ("c", copy),
            ("a", "auto-join"),
            ("d", "forget"),
        ]
    } else if open {
        let qr = if d.open_qr {
            "hide QR code"
        } else {
            "show QR code"
        };
        vec![("y", qr), ("c", "copy ip"), ("esc", "close")]
    } else {
        vec![("c", "copy ip"), ("esc", "close")]
    }
}

fn details_footer(
    app: &App,
    inner: Rect,
    row: usize,
    d: &DetailsDialog,
    hint: Vec<(&'static str, &'static str)>,
) -> Line<'static> {
    if !d.confirm_forget {
        return dialog_hint(app, inner, row, &hint);
    }
    let name = d.network.ssid.clone().unwrap_or_default();
    Line::from(vec![
        Span::styled(format!(" Forget {name}?   "), theme::warn()),
        Span::styled("y", theme::accent()),
        Span::styled(" yes   ", theme::dim()),
        Span::styled("n", theme::accent()),
        Span::styled(" no", theme::dim()),
    ])
}

/// The code as text lines, or why there is none.
fn qr_lines(code: Option<Vec<String>>) -> Vec<Line<'static>> {
    match code {
        Some(code) => code
            .into_iter()
            .map(|l| Line::styled(format!(" {l}"), Style::new().fg(theme::FG)))
            .collect(),
        None => vec![Line::styled(
            "   The password is too long for a QR code.",
            theme::err(),
        )],
    }
}

/// Details without room for the code: the password above the code alone.
fn draw_share(
    app: &App,
    frame: &mut Frame,
    name: &str,
    password: &str,
    code: Option<Vec<String>>,
    hint: &[(&str, &str)],
) {
    let code = qr_lines(code);
    let width = code.first().map_or(0, Line::width) as u16;
    let height = code.len() as u16 + 4;
    let inner = open_dialog(
        app,
        frame,
        &format!("Share {name}"),
        (width + 4).max(66),
        height,
    );
    let mut lines = vec![Line::from(vec![
        Span::styled("   password  ", theme::dim()),
        Span::raw(password.to_string()),
    ])];
    let pad = " ".repeat(inner.width.saturating_sub(width) as usize / 2);
    lines.extend(code.into_iter().map(|mut line| {
        line.spans.insert(0, Span::raw(pad.clone()));
        line
    }));
    let row = lines.len();
    lines.push(dialog_hint(app, inner, row, hint));
    widgets::text(frame, inner, lines);
}

fn draw_ipv4(app: &App, frame: &mut Frame, d: &Ipv4Form) {
    let inner = open_dialog(app, frame, &format!("IPv4 · {}", d.name), 50, 13);
    let option = |text: &str, chosen: bool| {
        let style = if chosen {
            Style::new().bg(theme::FIELD).fg(theme::BLUE).bold()
        } else {
            theme::dim()
        };
        Span::styled(format!(" {text} "), style)
    };
    let mode = Line::from(vec![
        Span::styled("  mode       ", theme::dim()),
        option("Automatic", !d.manual),
        Span::raw("  "),
        option("Manual", d.manual),
    ]);
    let field = |label: &str, input: &TextInput, index: usize| {
        if d.manual {
            field_line(label, input, 22, d.focus == index)
        } else {
            Line::from(vec![
                Span::styled(format!("  {}", fit(label, 10)), theme::dim()),
                Span::styled(input.value.clone(), theme::faint()),
            ])
        }
    };
    let note = match (&d.error, app.snapshot.caps.edit_needs_admin) {
        (Some(error), _) => Line::styled(format!("  {error}"), theme::err()),
        (None, true) => Line::styled("  saving asks for your admin password", theme::dim()),
        (None, false) => Line::raw(""),
    };
    let lines = vec![
        Line::raw(""),
        mode,
        Line::raw(""),
        field("address", &d.address, 1),
        field("subnet", &d.subnet, 2),
        field("router", &d.router, 3),
        Line::raw(""),
        note,
        Line::raw(""),
        dialog_hint(
            app,
            inner,
            9,
            &[("↵", "save"), ("tab", "next field"), ("esc", "cancel")],
        ),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_dns(app: &App, frame: &mut Frame, d: &DnsForm) {
    let inner = open_dialog(app, frame, &format!("DNS · {}", d.name), 56, 11);
    let note = match (&d.error, app.snapshot.caps.edit_needs_admin) {
        (Some(error), _) => Line::styled(format!("  {error}"), theme::err()),
        (None, true) => Line::styled("  saving asks for your admin password", theme::dim()),
        (None, false) => Line::raw(""),
    };
    let lines = vec![
        Line::raw(""),
        field_line("servers", &d.input, 36, true),
        Line::styled(
            "             comma-separated, empty = automatic",
            theme::dim(),
        ),
        Line::raw(""),
        note,
        Line::raw(""),
        dialog_hint(app, inner, 6, &[("↵", "save"), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_help(app: &App, frame: &mut Frame) {
    let keys = help_lines(app);
    let inner = open_dialog(app, frame, "Keys", 58, keys.len() as u16 + 4);
    let mut lines = vec![Line::raw("")];
    lines.extend(keys.into_iter().map(|(key, what)| {
        Line::from(vec![
            Span::styled(format!("  {}", fit(key, 10)), theme::accent()),
            Span::styled(what, theme::text()),
        ])
    }));
    widgets::text(frame, inner, lines);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Cmd, Event, mock};
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use telmo_kit::App as _;
    use telmo_speed::{Record, Update};
    use tokio::sync::mpsc::unbounded_channel;

    fn app() -> App {
        let (cmds, _) = unbounded_channel();
        let (events, _) = unbounded_channel();
        App::new(mock::data(), cmds, events, true)
    }

    fn press(app: &mut App, keys: &str) {
        for c in keys.chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn code(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn render(app: &App) -> String {
        telmo_kit::test::render(90, 22, |f| draw(app, f))
    }

    #[test]
    fn wifi() {
        insta::assert_snapshot!(render(&app()));
    }

    #[test]
    fn join() {
        let mut app = app();
        press(&mut app, "jjj");
        code(&mut app, KeyCode::Enter);
        press(&mut app, "hunter22xx");
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn details() {
        let mut app = app();
        press(&mut app, "i");
        app.event(Event::Details(crate::model::Details {
            public_ip: Some("203.0.113.24".to_string()),
            ..Default::default()
        }));
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn qr() {
        let mut app = app();
        press(&mut app, "iy");
        app.event(Event::Password {
            ssid: "HomeNet-5G".to_string(),
            password: Ok("correct-horse-battery".to_string()),
        });
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn ethernet() {
        let mut app = app();
        press(&mut app, "hj");
        code(&mut app, KeyCode::Tab);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn ipv4() {
        let mut app = app();
        press(&mut app, "hjl");
        code(&mut app, KeyCode::Enter);
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn vpn() {
        let mut app = app();
        press(&mut app, "hjjjjl");
        insta::assert_snapshot!(render(&app));
    }

    fn speed_app() -> App {
        let mut app = app();
        press(&mut app, "s");
        app.speed.last = Some(Record {
            at: telmo_kit::time::unix_now() - 3 * 3600 - 60,
            network: Some("HomeNet-5G".to_string()),
            down_mbps: 388.0,
            up_mbps: 41.2,
            latency_ms: 9.0,
        });
        app
    }

    fn feed(app: &mut App, updates: Vec<Update>) {
        for update in updates {
            app.event(Event::Speed(update));
        }
    }

    fn download_done() -> Vec<Update> {
        let progress = |phase, mbps, fraction| Update::Progress {
            phase,
            mbps,
            fraction,
        };
        vec![
            Update::Latency {
                idle_ms: 8.0,
                jitter_ms: 1.2,
            },
            progress(Phase::Download, 120.0, 0.2),
            progress(Phase::Download, 300.0, 0.5),
            progress(Phase::Download, 410.0, 0.8),
            Update::Result {
                phase: Phase::Download,
                mbps: 412.6,
                loaded_ms: None,
            },
        ]
    }

    #[tokio::test(flavor = "current_thread")]
    async fn speed() {
        let mut app = speed_app();
        feed(&mut app, download_done());
        feed(
            &mut app,
            vec![
                Update::Progress {
                    phase: Phase::Upload,
                    mbps: 20.0,
                    fraction: 0.3,
                },
                Update::Progress {
                    phase: Phase::Upload,
                    mbps: 38.0,
                    fraction: 0.6,
                },
            ],
        );
        insta::assert_snapshot!(render(&app));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn speeddone() {
        let mut app = speed_app();
        feed(&mut app, download_done());
        feed(
            &mut app,
            vec![
                Update::Progress {
                    phase: Phase::Upload,
                    mbps: 38.0,
                    fraction: 0.6,
                },
                Update::Result {
                    phase: Phase::Upload,
                    mbps: 39.4,
                    loaded_ms: Some(24.0),
                },
                Update::Finished,
            ],
        );
        press(&mut app, "c");
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn error() {
        let mut app = app();
        press(&mut app, "jjj");
        app.event(Event::Done {
            target: "Tanaka-AP".to_string(),
            result: Err("Wrong password for Tanaka-AP.".to_string()),
        });
        insta::assert_snapshot!(render(&app));
    }

    #[test]
    fn location() {
        let mut snapshot = mock::data();
        if let Some(wifi) = snapshot.wifi.as_mut() {
            wifi.names_hidden = true;
            wifi.networks.truncate(5);
            wifi.saved_elsewhere.clear();
            for n in wifi.networks.iter_mut() {
                n.ssid = None;
            }
            wifi.networks[3].security = Security::Open;
        }
        let mut app = app();
        app.event(Event::Snapshot(snapshot));
        insta::assert_snapshot!(render(&app));
    }

    /// Cell of the first match of `text` on the drawn screen.
    fn find(app: &App, text: &str) -> (u16, u16) {
        for (row, line) in render(app).lines().enumerate() {
            if let Some(byte) = line.find(text) {
                return (line[..byte].chars().count() as u16, row as u16);
            }
        }
        panic!("{text:?} isn't on screen");
    }

    fn mouse(app: &mut App, kind: MouseEventKind, text: &str) {
        let (column, row) = find(app, text);
        app.mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn click(app: &mut App, text: &str) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), text);
    }

    #[test]
    fn clicking_a_row_selects_it_and_clicking_again_acts() {
        let mut app = app();
        click(&mut app, "Tanaka-AP");
        assert_eq!(app.row, 3);
        assert!(app.dialog.is_none());
        click(&mut app, "Tanaka-AP");
        assert!(matches!(app.dialog, Some(Dialog::Join(_))));
    }

    #[test]
    fn clicking_the_sidebar_selects_an_interface() {
        let mut app = app();
        click(&mut app, "Ethernet");
        assert_eq!((app.sel, app.focus), (1, Focus::Sidebar));
        assert_eq!(app.pane(), Pane::Wired(1));
        click(&mut app, "Ethernet");
        assert_eq!(app.focus, Focus::Pane);
    }

    #[test]
    fn the_wheel_moves_the_selection_under_the_cursor() {
        let mut app = app();
        let down = MouseEventKind::ScrollDown;
        mouse(&mut app, down, "HomeNet-5G");
        mouse(&mut app, down, "HomeNet-5G");
        assert_eq!(app.row, 2);
        mouse(&mut app, MouseEventKind::ScrollUp, "HomeNet-5G");
        assert_eq!(app.row, 1);
        mouse(&mut app, down, "Ethernet");
        assert_eq!((app.sel, app.focus), (1, Focus::Sidebar));
    }

    #[test]
    fn clicking_the_key_bar_presses_the_key() {
        let mut app = app();
        click(&mut app, "rescan");
        assert!(app.pending.contains("wifi"));
        click(&mut app, "more");
        assert!(matches!(app.dialog, Some(Dialog::Help)));
    }

    #[test]
    fn clicking_a_dialog_hint_presses_the_key_and_outside_closes() {
        let mut app = app();
        press(&mut app, "i");
        click(&mut app, "copy ip");
        assert!(app.toast.is_some());
        assert!(app.dialog.is_some());
        click(&mut app, "signal");
        assert!(app.dialog.is_some());
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.dialog.is_none());
    }

    #[test]
    fn a_toggles_auto_join_in_details() {
        let (cmds, mut sent) = unbounded_channel();
        let (events, _) = unbounded_channel();
        let mut app = App::new(mock::data(), cmds, events, true);
        press(&mut app, "i");
        assert!(render(&app).contains("auto-join  on"));
        click(&mut app, "auto-join  on");
        assert!(matches!(sent.try_recv(), Ok(Cmd::Details)));
        assert!(matches!(
            sent.try_recv(),
            Ok(Cmd::SetAutoJoin { ssid, on: false }) if ssid == "HomeNet-5G"
        ));
    }

    #[test]
    fn showing_the_password_shows_the_code_too() {
        let mut app = app();
        press(&mut app, "i");
        click(&mut app, "show password");
        app.event(Event::Password {
            ssid: "HomeNet-5G".to_string(),
            password: Ok("correct-horse-battery".to_string()),
        });
        let screen = render(&app);
        assert!(screen.contains("Share HomeNet-5G"));
        assert!(screen.contains("correct-horse-battery"));
        assert!(!screen.contains("QR code"));
    }
}
