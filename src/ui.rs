use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, Gauge, List, ListItem, ListState, Padding, Paragraph, Row,
        Table, TableState, Tabs, Wrap,
    },
};

use crate::{
    app::{App, Tab},
    config::AccountKind,
};

const ACCENT: Color = Color::Rgb(92, 184, 92);
const SECONDARY: Color = Color::Rgb(82, 168, 201);
const MUTED: Color = Color::Rgb(135, 145, 150);
const WARNING: Color = Color::Rgb(232, 178, 72);
const ERROR: Color = Color::Rgb(224, 90, 90);
const SURFACE: Color = Color::Rgb(27, 31, 32);

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(17, 20, 20))),
        area,
    );
    let layout = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .split(area);
    draw_header(frame, app, layout[0]);
    match app.tab {
        Tab::Play => draw_play(frame, app, layout[1]),
        Tab::Versions => draw_versions(frame, app, layout[1]),
        Tab::Accounts => draw_accounts(frame, app, layout[1]),
        Tab::Settings => draw_settings(frame, app, layout[1]),
        Tab::Logs => draw_logs(frame, app, layout[1]),
    }
    draw_status(frame, app, layout[2]);
    if app.modal.is_some() {
        draw_modal(frame, app, area);
    } else if app.version_deletion.is_some() {
        draw_version_deletion(frame, app, area);
    } else if app.device_login.is_some() {
        draw_device_login(frame, app, area);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::horizontal([Constraint::Length(14), Constraint::Min(20)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "M",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "ICRC",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(Block::default().padding(Padding::new(2, 0, 1, 0))),
        columns[0],
    );
    let selected = Tab::ALL.iter().position(|tab| tab == &app.tab).unwrap_or(0);
    let titles = Tab::ALL
        .iter()
        .map(|tab| Line::from(format!(" {} ", tab.title())))
        .collect::<Vec<_>>();
    frame.render_widget(
        Tabs::new(titles)
            .select(selected)
            .style(Style::default().fg(MUTED))
            .highlight_style(
                Style::default()
                    .fg(Color::White)
                    .bg(SURFACE)
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::raw(" "))
            .block(Block::default().padding(Padding::new(0, 1, 1, 0))),
        columns[1],
    );
}

fn draw_play(frame: &mut Frame, app: &App, area: Rect) {
    let is_narrow = area.width < 84;
    let direction = if is_narrow {
        Direction::Vertical
    } else {
        Direction::Horizontal
    };
    let panel_constraints = if is_narrow {
        [Constraint::Percentage(45), Constraint::Percentage(55)]
    } else {
        [Constraint::Percentage(62), Constraint::Percentage(38)]
    };
    let panels = Layout::default()
        .direction(direction)
        .constraints(panel_constraints)
        .margin(1)
        .split(area);

    let items = if app.installed.is_empty() {
        vec![ListItem::new(Line::styled(
            "No installed versions",
            Style::default().fg(MUTED),
        ))]
    } else {
        app.installed
            .iter()
            .map(|version| {
                let selected = app.config.selected_version.as_deref() == Some(version.as_str());
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if selected { "ACTIVE  " } else { "        " },
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(version, Style::default().fg(Color::White)),
                ]))
            })
            .collect()
    };
    let mut version_state = ListState::default();
    if !app.installed.is_empty() {
        version_state.select(Some(app.play_version_index));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(panel("Installed versions"))
            .highlight_style(Style::default().bg(SURFACE).fg(Color::White))
            .highlight_symbol("  "),
        panels[0],
        &mut version_state,
    );

    let side = if is_narrow {
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(panels[1])
    } else {
        Layout::vertical([Constraint::Length(7), Constraint::Min(6)]).split(panels[1])
    };
    let account = app.config.accounts.get(app.account_index);
    let account_text = account.map_or_else(
        || Text::from(Line::styled("No account", Style::default().fg(MUTED))),
        |account| {
            Text::from(vec![
                Line::styled(
                    &account.username,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    match account.kind {
                        AccountKind::Offline => "Offline profile",
                        AccountKind::Microsoft => "Microsoft profile",
                    },
                    Style::default().fg(SECONDARY),
                ),
                Line::styled(short_id(&account.id), Style::default().fg(MUTED)),
            ])
        },
    );
    frame.render_widget(
        Paragraph::new(account_text).block(panel("Account")),
        side[0],
    );

    let selected_version = app
        .installed
        .get(app.play_version_index)
        .map(String::as_str)
        .unwrap_or("None");
    let state = if app.game_running {
        Span::styled(
            "RUNNING",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "READY",
            Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD),
        )
    };
    let details = Text::from(vec![
        Line::from(vec![Span::styled("State      ", muted()), state]),
        Line::from(vec![
            Span::styled("Version    ", muted()),
            Span::raw(selected_version),
        ]),
        Line::from(vec![
            Span::styled("Memory     ", muted()),
            Span::raw(format!(
                "{}-{} MB",
                app.config.settings.min_memory_mb, app.config.settings.max_memory_mb
            )),
        ]),
        Line::from(vec![
            Span::styled("Resolution ", muted()),
            Span::raw(format!(
                "{} x {}",
                app.config.settings.width, app.config.settings.height
            )),
        ]),
        Line::from(vec![
            Span::styled("Java       ", muted()),
            Span::raw(&app.config.settings.java_path),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(details)
            .block(panel("Launch profile"))
            .wrap(Wrap { trim: false }),
        side[1],
    );
}

fn draw_versions(frame: &mut Frame, app: &App, area: Rect) {
    let versions = app.visible_versions();
    let latest_release = app
        .manifest
        .as_ref()
        .map(|value| value.latest.release.as_str());
    let latest_snapshot = app
        .manifest
        .as_ref()
        .map(|value| value.latest.snapshot.as_str());
    let rows = versions.iter().map(|version| {
        let installed = app.installed.iter().any(|id| id == &version.id);
        let channel = if Some(version.id.as_str()) == latest_release {
            "latest release"
        } else if Some(version.id.as_str()) == latest_snapshot {
            "latest snapshot"
        } else {
            version.kind.as_str()
        };
        Row::new(vec![
            Cell::from(if installed { "INSTALLED" } else { "" }),
            Cell::from(version.id.as_str()),
            Cell::from(channel),
            Cell::from(
                version
                    .release_time
                    .get(..10)
                    .unwrap_or(&version.release_time),
            ),
        ])
        .style(if installed {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(Color::White)
        })
    });
    let mut state = TableState::default();
    if !versions.is_empty() {
        state.select(Some(app.version_index));
    }
    let title = if app.config.settings.show_snapshots {
        "Minecraft catalog: all channels"
    } else {
        "Minecraft catalog: releases"
    };
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Percentage(36),
                Constraint::Percentage(34),
                Constraint::Length(12),
            ],
        )
        .header(
            Row::new(["Status", "Version", "Channel", "Released"])
                .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(panel(title))
        .row_highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD))
        .highlight_symbol("  "),
        inner_margin(area),
        &mut state,
    );
}

fn draw_accounts(frame: &mut Frame, app: &App, area: Rect) {
    let rows = app.config.accounts.iter().map(|account| {
        let selected = app.config.selected_account.as_deref() == Some(account.id.as_str());
        let kind = match account.kind {
            AccountKind::Offline => "Offline",
            AccountKind::Microsoft => "Microsoft",
        };
        Row::new(vec![
            Cell::from(if selected { "ACTIVE" } else { "" }),
            Cell::from(account.username.as_str()),
            Cell::from(kind),
            Cell::from(short_id(&account.id)),
        ])
    });
    let mut state = TableState::default();
    if !app.config.accounts.is_empty() {
        state.select(Some(app.account_index));
    }
    frame.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Percentage(35),
                Constraint::Length(14),
                Constraint::Min(20),
            ],
        )
        .header(
            Row::new(["Status", "Username", "Type", "Profile ID"])
                .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(panel("Profiles"))
        .row_highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD))
        .highlight_symbol("  "),
        inner_margin(area),
        &mut state,
    );
}

fn draw_settings(frame: &mut Frame, app: &App, area: Rect) {
    let settings = &app.config.settings;
    let data = [
        ("Java executable", settings.java_path.clone()),
        ("Minimum memory", format!("{} MB", settings.min_memory_mb)),
        ("Maximum memory", format!("{} MB", settings.max_memory_mb)),
        ("Window width", settings.width.to_string()),
        ("Window height", settings.height.to_string()),
        ("Extra JVM arguments", settings.extra_jvm_args.join(" ")),
        (
            "Microsoft client ID",
            if settings.microsoft_client_id.is_empty() {
                "Not configured".to_owned()
            } else {
                settings.microsoft_client_id.clone()
            },
        ),
        (
            "Snapshot versions",
            if settings.show_snapshots {
                "Enabled"
            } else {
                "Disabled"
            }
            .to_owned(),
        ),
    ];
    let rows = data
        .into_iter()
        .map(|(name, value)| Row::new(vec![Cell::from(name), Cell::from(value)]));
    let mut state = TableState::default().with_selected(app.setting_index);
    frame.render_stateful_widget(
        Table::new(
            rows,
            [Constraint::Percentage(36), Constraint::Percentage(64)],
        )
        .header(
            Row::new(["Setting", "Value"])
                .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD))
                .bottom_margin(1),
        )
        .block(panel("Launcher settings"))
        .row_highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD))
        .highlight_symbol("  "),
        inner_margin(area),
        &mut state,
    );
}

fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let text = if app.logs.is_empty() {
        Text::from(Line::styled("No game output", Style::default().fg(MUTED)))
    } else {
        Text::from(
            app.logs
                .iter()
                .map(|line| Line::raw(line.as_str()))
                .collect::<Vec<_>>(),
        )
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(panel("Game output"))
            .scroll((app.log_scroll.min(u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false }),
        inner_margin(area),
    );
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(SURFACE))
        .padding(Padding::horizontal(2));
    if let Some((completed, total, label)) = &app.progress {
        let ratio = if *total == 0 {
            0.0
        } else {
            *completed as f64 / *total as f64
        };
        frame.render_widget(
            Gauge::default()
                .block(block)
                .gauge_style(Style::default().fg(ACCENT).bg(SURFACE))
                .ratio(ratio.clamp(0.0, 1.0))
                .label(format!("{label}  {completed}/{total}")),
            area,
        );
    } else {
        let style = if app.status.starts_with("Error:") || app.status.starts_with("Failed") {
            Style::default().fg(ERROR)
        } else if app.active_task || app.game_running {
            Style::default().fg(WARNING)
        } else {
            Style::default().fg(MUTED)
        };
        frame.render_widget(
            Paragraph::new(Line::styled(&app.status, style))
                .block(block)
                .alignment(Alignment::Left),
            area,
        );
    }
}

fn draw_modal(frame: &mut Frame, app: &App, area: Rect) {
    let Some(modal) = &app.modal else {
        return;
    };
    let popup = centered(area, 70.min(area.width.saturating_sub(2)), 7);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(Color::Rgb(17, 20, 20)))
            .title(format!(" {} ", modal.title)),
        popup,
    );
    let inner = Rect::new(
        popup.x.saturating_add(2),
        popup.y.saturating_add(2),
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(3),
    );
    let lines = vec![
        Line::styled(&modal.value, Style::default().fg(Color::White)),
        Line::styled(
            modal.error.as_deref().unwrap_or(""),
            Style::default().fg(ERROR),
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
    let cursor_offset = modal
        .value
        .chars()
        .count()
        .min(inner.width.saturating_sub(1) as usize) as u16;
    frame.set_cursor_position((inner.x + cursor_offset, inner.y));
}

fn draw_device_login(frame: &mut Frame, app: &App, area: Rect) {
    let Some(login) = &app.device_login else {
        return;
    };
    let width = 76.min(area.width.saturating_sub(2));
    let popup = centered(area, width, 14.min(area.height));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(SECONDARY))
            .style(Style::default().bg(Color::Rgb(17, 20, 20)))
            .title(" Microsoft login ")
            .padding(Padding::horizontal(2)),
        popup,
    );
    let inner = Rect::new(
        popup.x.saturating_add(3),
        popup.y.saturating_add(2),
        popup.width.saturating_sub(6),
        popup.height.saturating_sub(4),
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::styled(
                &login.browser_status,
                Style::default().fg(if login.browser_status.starts_with("Browser opened") {
                    ACCENT
                } else {
                    WARNING
                }),
            ),
            Line::raw(""),
            Line::styled(&login.message, Style::default().fg(MUTED)),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Code  ", muted()),
                Span::styled(
                    &login.user_code,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("URL   ", muted()),
                Span::styled(&login.verification_uri, Style::default().fg(SECONDARY)),
            ]),
        ]))
        .wrap(Wrap { trim: true }),
        inner,
    );
}

fn draw_version_deletion(frame: &mut Frame, app: &App, area: Rect) {
    let Some(confirmation) = &app.version_deletion else {
        return;
    };
    let width = 62.min(area.width.saturating_sub(2));
    let popup = centered(area, width, 8.min(area.height));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ERROR))
            .style(Style::default().bg(Color::Rgb(17, 20, 20)))
            .title(" Delete Minecraft version ")
            .padding(Padding::horizontal(2)),
        popup,
    );
    let inner = Rect::new(
        popup.x.saturating_add(3),
        popup.y.saturating_add(2),
        popup.width.saturating_sub(6),
        popup.height.saturating_sub(4),
    );
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::raw("Delete Minecraft "),
                Span::styled(
                    &confirmation.version_id,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" from this device?"),
            ]),
            Line::styled("This cannot be undone.", Style::default().fg(WARNING)),
        ]))
        .wrap(Wrap { trim: true }),
        inner,
    );
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SURFACE))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1))
}

fn inner_margin(area: Rect) -> Rect {
    Layout::vertical([Constraint::Min(1)]).margin(1).split(area)[0]
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn muted() -> Style {
    Style::default().fg(MUTED)
}

fn short_id(id: &str) -> &str {
    id.get(..18).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::config::ConfigStore;

    #[tokio::test]
    async fn renders_wide_and_narrow_terminals() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let app = App::load(ConfigStore::at(temporary.path().join("micrc")))
            .await
            .unwrap();
        for (width, height) in [(120, 35), (60, 20)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let content = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(content.contains("MICRC"));
            assert!(content.contains("Play"));
            assert!(content.contains("Player"));
            assert!(content.contains("Java"));
        }
    }

    #[tokio::test]
    async fn renders_version_deletion_confirmation() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let mut app = App::load(ConfigStore::at(temporary.path().join("micrc")))
            .await
            .unwrap();
        app.version_deletion = Some(crate::app::DeleteVersionConfirmation {
            version_id: "1.21.8".to_owned(),
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(content.contains("Delete Minecraft version"));
        assert!(content.contains("1.21.8"));
        assert!(content.contains("cannot be undone"));
    }

    #[tokio::test]
    async fn renders_browser_login_status_and_fallback_details() {
        let temporary = temp_dir::TempDir::new().unwrap();
        let mut app = App::load(ConfigStore::at(temporary.path().join("micrc")))
            .await
            .unwrap();
        app.device_login = Some(crate::app::DeviceLogin {
            user_code: "ABCD-EFGH".to_owned(),
            verification_uri: "https://microsoft.com/link".to_owned(),
            message: "Complete the sign-in request.".to_owned(),
            browser_status: "Browser opened. Complete the sign-in there.".to_owned(),
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let content = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(content.contains("Browser opened"));
        assert!(content.contains("ABCD-EFGH"));
        assert!(content.contains("https://microsoft.com/link"));
    }
}
