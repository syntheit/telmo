//! A fixed app list for `--mock` and the snapshot tests.

use super::{AppRow, sort_rows};

const MB: u64 = 1024 * 1024;

pub fn rows() -> Vec<AppRow> {
    let row = |pid, name: &str, memory: u64, cpu: f32, unresponsive| AppRow {
        pid,
        name: name.into(),
        memory,
        cpu: Some(cpu),
        unresponsive,
        windows: 0,
    };
    let mut rows = vec![
        row(101, "Telegram", 380 * MB, 0.0, true),
        row(102, "Zen", 1946 * MB, 12.0, false),
        row(103, "Spotify", 640 * MB, 3.0, false),
        row(104, "Messages", 210 * MB, 0.0, false),
        row(105, "Finder", 150 * MB, 1.0, false),
        row(106, "Ghostty", 120 * MB, 2.0, false),
        row(107, "Notes", 95 * MB, 0.0, false),
        row(108, "Slack", 870 * MB, 6.0, false),
    ];
    sort_rows(&mut rows);
    rows
}
