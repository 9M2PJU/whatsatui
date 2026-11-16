use super::*;
use crate::app::update::attachments::{browser_listed, browser_visible, format_entry_size};
pub(super) fn dialog(
    frame: &mut Frame,
    area: Rect,
    view: &ViewModel,
    config: &Config,
    hits: &mut InteractionMap,
) {
    let Some(Overlay::Attachment {
        editor,
        selected,
        importing,
        error,
    }) = &view.overlay
    else {
        return;
    };
    let show_saved = !browser_visible(editor.text(), !view.draft.recovered.is_empty())
        && !view.draft.recovered.is_empty();
    let (browser_base, browser_entries) =
        if browser_visible(editor.text(), !view.draft.recovered.is_empty()) {
            let (base, entries) = browser_listed(editor.text());
            (Some(base), entries)
        } else {
            (None, Vec::new())
        };
    let browser_count = browser_entries.len().min(8) as u16;
    let saved_count = if show_saved {
        view.draft.recovered.len().min(5) as u16
    } else {
        0
    };
    // Browser rows plus an optional current-directory line.
    let list_count = if browser_base.is_some() {
        browser_count
    } else {
        saved_count
    };
    let extra = u16::from(show_saved || browser_base.is_some());
    let rect = overlays::centered(area, 76, 8 + list_count + extra);
    frame.render_widget(ratatui::widgets::Clear, rect);
    let border = block(" Attach image or file ".into(), true, view, config);
    let inner = border.inner(rect);
    frame.render_widget(border, rect);
    hits.popup(frame, rect, view, config);
    let hint = "File path or picker · ↑/↓ selects · Enter opens/attaches · up to 50 MiB";
    frame.render_widget(
        Paragraph::new(hint).style(style(config, view, ThemeRole::Inactive)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    search::query(
        frame,
        Rect::new(inner.x, inner.y + 2, inner.width, 1),
        editor,
        view,
        config,
        hits,
    );
    let status = if importing.is_some() {
        "Preparing file…"
    } else {
        error
            .as_deref()
            .unwrap_or("Enter attaches · Esc cancels · ~/ and spaces supported")
    };
    let mut status_y: u16;
    if let Some(base) = &browser_base {
        frame.render_widget(
            Paragraph::new(format!("Files in {} · .. goes up", base.display())).style(style(
                config,
                view,
                ThemeRole::Inactive,
            )),
            Rect::new(inner.x, inner.y + 4, inner.width, 1),
        );
        let start = view
            .list_offsets
            .get(&Context::Attachment)
            .copied()
            .unwrap_or(0)
            .min(*selected)
            .max(
                selected
                    .saturating_add(1)
                    .saturating_sub(browser_count.max(1) as usize),
            );
        let rows = browser_entries
            .iter()
            .enumerate()
            .skip(start)
            .take(browser_count as usize)
            .map(|(i, entry)| {
                let label = if entry.is_dir {
                    format!("{}/", single(&entry.name))
                } else {
                    format!(
                        "{} · {}",
                        single(&entry.name),
                        format_entry_size(entry.size)
                    )
                };
                Line::styled(
                    single(&format!(
                        "{} {label}",
                        if i == *selected { "›" } else { " " },
                    )),
                    style(
                        config,
                        view,
                        if i == *selected {
                            ThemeRole::Accent
                        } else {
                            ThemeRole::Inactive
                        },
                    ),
                )
            })
            .collect::<Vec<_>>();
        let rows_text = if rows.is_empty() {
            vec![Line::styled(
                "No matching files here · type another path or pick ..",
                style(config, view, ThemeRole::Inactive),
            )]
        } else {
            rows
        };
        frame.render_widget(
            Paragraph::new(rows_text),
            Rect::new(
                inner.x,
                inner.y + 5,
                inner.width,
                browser_count.min(inner.height.saturating_sub(5)),
            ),
        );
        if !browser_entries.is_empty() {
            hits.list(
                Context::Attachment,
                Rect::new(
                    inner.x,
                    inner.y + 5,
                    inner.width,
                    browser_count.min(inner.height.saturating_sub(5)),
                ),
                start,
                browser_entries.iter().map(|_| 1),
                Target::Menu,
            );
        }
        status_y = 5 + browser_count;
    } else if show_saved {
        frame.render_widget(
            Paragraph::new("Saved drafts · ↑/↓ selects · Enter restores · type a path to attach"),
            Rect::new(inner.x, inner.y + 4, inner.width, 1),
        );
        let start = view
            .list_offsets
            .get(&Context::Attachment)
            .copied()
            .unwrap_or(0)
            .min(*selected)
            .max((selected + 1).saturating_sub(saved_count.max(1) as usize));
        let rows = view
            .draft
            .recovered
            .iter()
            .enumerate()
            .skip(start)
            .take(saved_count as usize)
            .map(|(i, draft)| {
                let filename = draft
                    .attachment
                    .as_ref()
                    .map(|a| a.filename.as_str())
                    .or(draft.document.as_ref().map(|d| d.filename.as_str()))
                    .unwrap_or("Text draft");
                Line::styled(
                    single(&format!(
                        "{} {filename} · {}",
                        if i == *selected { "›" } else { " " },
                        draft.text
                    )),
                    style(
                        config,
                        view,
                        if i == *selected {
                            ThemeRole::Accent
                        } else {
                            ThemeRole::Inactive
                        },
                    ),
                )
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(rows),
            Rect::new(
                inner.x,
                inner.y + 5,
                inner.width,
                saved_count.min(inner.height.saturating_sub(5)),
            ),
        );
        hits.list(
            Context::Attachment,
            Rect::new(
                inner.x,
                inner.y + 5,
                inner.width,
                saved_count.min(inner.height.saturating_sub(5)),
            ),
            start,
            view.draft.recovered.iter().map(|_| 1),
            Target::Menu,
        );
        status_y = 5 + saved_count;
    } else {
        status_y = 4;
    };
    frame.render_widget(
        Paragraph::new(single(status))
            .wrap(Wrap { trim: false })
            .style(style(
                config,
                view,
                if error.is_some() {
                    ThemeRole::Error
                } else {
                    ThemeRole::Hints
                },
            )),
        Rect::new(
            inner.x,
            inner.y + status_y,
            inner.width,
            inner.height.saturating_sub(status_y),
        ),
    );
}
