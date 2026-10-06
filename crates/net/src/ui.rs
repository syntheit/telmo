//! Drawing for telmo-net: a pure function of the app state.

use crate::app::{
    App, DetailsDialog, Dialog, DnsForm, Focus, Ipv4Form, Join, Meter, Pane, Qr, Screen, WifiRow,
};
use crate::model::*;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
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
    widgets::keys(frame, screen.keys, &key_bar(app));
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

fn key_bar(app: &App) -> Vec<(&'static str, &'static str)> {
    let mut keys: Vec<(&str, &str)> = match app.screen {
        Screen::Speed => return vec![("↵", "run again"), ("c", "copy result"), ("esc", "back")],
        Screen::Main if app.focus == Focus::Sidebar => {
            vec![("↵", "open"), ("s", "speedtest"), ("tab", "pane")]
        }
        Screen::Main => match app.pane() {
            Pane::Wifi if wifi_off(app) => vec![("p", "turn on"), ("s", "speedtest")],
            Pane::Wifi => vec![("↵", "join"), ("i", "details"), ("s", "speedtest")],
            Pane::Wired(i) if app.has_settings(&app.snapshot.interfaces[i]) => {
                vec![("↵", "edit")]
            }
            Pane::Vpn => vec![("↵", "connect/disconnect"), ("i", "details")],
            Pane::Wired(_) | Pane::Empty => vec![("s", "speedtest")],
        },
    };
    if app.screen == Screen::Main && app.focus == Focus::Pane {
        keys.push(("tab", "interfaces"));
    }
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
    ];
    match app.pane() {
        Pane::Wifi => lines.extend([
            ("↵", "join the selected network"),
            ("i", "details, password, QR, forget"),
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
    widgets::rows(frame, inner, lines, Some(app.sel * 3));
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
    let intro = vec![
        Line::styled("macOS hides network names until Location is", theme::text()),
        Line::styled("allowed for Telmo.", theme::text()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("L", theme::accent()),
            Span::styled(" open Location Services settings", theme::dim()),
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
    widgets::rows(frame, area, lines, selected);
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
    if app.has_settings(interface) {
        let rule = "─".repeat((inner.width as usize).saturating_sub(16));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled("settings ", theme::dim()),
            Span::styled(rule, theme::faint()),
        ]));
        if app.focus == Focus::Pane {
            selected = Some(lines.len() + app.row);
        }
        lines.extend(setting_lines(app, interface));
    }
    widgets::rows(frame, inner, lines, selected);
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
    widgets::rows(frame, inner, lines, selected);
}

// --- speedtest ---

fn draw_speed(app: &App, frame: &mut Frame, body: Rect) {
    let [meters, latency, _, last, _] = Layout::vertical([
        Constraint::Length(9),
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
    digits[2].spans.push(Span::styled("  Mbps", theme::dim()));
    for mut row in digits {
        row.spans.insert(0, Span::raw("   "));
        lines.push(row);
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!("   {}", sparkline(&meter.samples, width.saturating_sub(6))),
        style,
    ));

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

/// The most recent samples that fit, scaled to the largest.
fn sparkline(samples: &[f64], width: usize) -> String {
    const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let recent = &samples[samples.len().saturating_sub(width)..];
    let max = recent.iter().copied().fold(0.0, f64::max);
    recent
        .iter()
        .map(|s| {
            let level = if max > 0.0 {
                (s / max * 7.0).round() as usize
            } else {
                0
            };
            LEVELS[level.min(7)]
        })
        .collect()
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = now.saturating_sub(at);
    match secs {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86400),
    }
}

// --- dialogs ---

fn draw_dialog(app: &App, frame: &mut Frame, dialog: &Dialog) {
    match dialog {
        Dialog::Join(d) => draw_join(frame, d),
        Dialog::Details(d) => draw_details(app, frame, d),
        Dialog::Qr(d) => draw_qr(frame, d),
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

fn draw_join(frame: &mut Frame, d: &Join) {
    let inner = widgets::dialog(frame, &d.title, 50, 7);
    let hint = widgets::hint(&[("↵", "join"), ("tab", "show password"), ("esc", "cancel")]);
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
    let title = d
        .network
        .ssid
        .clone()
        .unwrap_or_else(|| "hidden network".to_string());
    let inner = widgets::dialog(frame, &title, 58, 12);
    let ipv4 = d.network.connected.then(|| app.wifi_ipv4()).flatten();
    let none = "—".to_string();
    let public_ip = match (&app.details, d.network.connected) {
        (Some(details), _) => details.public_ip.clone().unwrap_or(none.clone()),
        (None, true) => "…".to_string(),
        (None, false) => none.clone(),
    };
    let password = match (&d.password, d.network.saved) {
        (Some(password), _) => password.clone(),
        (None, true) => "••••••••••".to_string(),
        (None, false) => "not saved".to_string(),
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
    let mut lines = vec![Line::raw("")];
    lines.extend(rows.map(|(label, value)| {
        Line::from(vec![
            Span::styled(format!("   {}", fit(label, 11)), theme::dim()),
            Span::raw(value),
        ])
    }));
    lines.push(Line::raw(""));
    lines.push(if d.confirm_forget {
        let name = d.network.ssid.clone().unwrap_or_default();
        Line::from(vec![
            Span::styled(format!(" Forget {name}?   "), theme::warn()),
            Span::styled("y", theme::accent()),
            Span::styled(" yes   ", theme::dim()),
            Span::styled("n", theme::accent()),
            Span::styled(" no", theme::dim()),
        ])
    } else if d.network.saved {
        let reveal = if d.password.is_some() {
            "hide password"
        } else {
            "show password"
        };
        widgets::hint(&[
            ("y", reveal),
            ("c", "copy ip"),
            ("Q", "share QR"),
            ("d", "forget"),
        ])
    } else {
        widgets::hint(&[("c", "copy ip"), ("esc", "close")])
    });
    widgets::text(frame, inner, lines);
}

fn draw_qr(frame: &mut Frame, d: &Qr) {
    let Some(code) = d.lines() else {
        let inner = widgets::dialog(frame, &d.ssid, 50, 5);
        let message = Line::styled(" The password is too long for a QR code.", theme::err());
        return widgets::text(frame, inner, vec![Line::raw(""), message]);
    };
    let width = code.first().map_or(0, |l| l.chars().count()) as u16;
    let inner = widgets::dialog(frame, &d.ssid, (width + 4).max(34), code.len() as u16 + 4);
    let mut lines: Vec<Line> = code
        .into_iter()
        .map(|l| Line::styled(format!(" {l}"), Style::new().fg(theme::FG)))
        .collect();
    lines.push(widgets::hint(&[("esc", "close")]));
    widgets::text(frame, inner, lines);
}

fn draw_ipv4(app: &App, frame: &mut Frame, d: &Ipv4Form) {
    let inner = widgets::dialog(frame, &format!("IPv4 · {}", d.name), 50, 13);
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
        widgets::hint(&[("↵", "save"), ("tab", "next field"), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_dns(app: &App, frame: &mut Frame, d: &DnsForm) {
    let inner = widgets::dialog(frame, &format!("DNS · {}", d.name), 56, 11);
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
        widgets::hint(&[("↵", "save"), ("esc", "cancel")]),
    ];
    widgets::text(frame, inner, lines);
}

fn draw_help(app: &App, frame: &mut Frame) {
    let keys = help_lines(app);
    let inner = widgets::dialog(frame, "Keys", 58, keys.len() as u16 + 4);
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
    use crate::backend::{Event, mock};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
        press(&mut app, "jj");
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
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
                - 3 * 3600
                - 60,
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
        press(&mut app, "jj");
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
}
