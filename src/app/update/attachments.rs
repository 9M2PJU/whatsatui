use super::*;

impl App {
    pub(super) fn open_attachment(&mut self) {
        self.clipboard_request = None;
        if self.view.loading || self.view.chat.is_none() || self.view.account.is_none() {
            return;
        }
        self.view.overlay = Some(Overlay::Attachment {
            editor: Editor::default(),
            selected: 0,
            importing: None,
            error: None,
        });
    }
    pub(super) fn import_attachment(&mut self, effects: &mut Vec<Effect>) {
        let Some(Overlay::Attachment {
            editor,
            selected,
            importing: None,
            ..
        }) = &self.view.overlay
        else {
            return;
        };
        let editor_text = editor.text().to_owned();
        let selected = *selected;
        let has_recovered = !self.view.draft.recovered.is_empty();
        // Empty input with saved drafts restores a draft (existing behavior).
        if editor_text.trim().is_empty() && !browser_visible(&editor_text, has_recovered) {
            if let Some(chat) = &self.view.chat
                && self.view.draft.restore(selected, chat)
            {
                self.editor = Editor::new(self.view.draft.text.clone());
                self.view.overlay = None;
                self.view.notice = Some("Saved draft restored; review before sending".into());
                self.remember();
            }
            return;
        }
        // Explorer selection wins when a file list is shown: Enter on a
        // directory navigates into it, Enter on a file attaches it.
        if browser_visible(&editor_text, has_recovered) {
            let (_, entries) = browser_listed(&editor_text);
            if let Some(entry) = entries.get(selected) {
                if entry.is_dir {
                    if let Some(Overlay::Attachment {
                        editor,
                        selected,
                        error,
                        ..
                    }) = &mut self.view.overlay
                    {
                        let mut next = entry.path.to_string_lossy().into_owned();
                        if !next.ends_with('/') {
                            next.push('/');
                        }
                        *editor = Editor::new(next);
                        *selected = 0;
                        *error = None;
                    }
                    return;
                }
                return self.attach_file_path(entry.path.to_string_lossy().into_owned(), effects);
            }
        }
        let path = clean_attachment_path(&editor_text);
        if path.is_empty() {
            return;
        }
        // Typing an existing directory navigates instead of erroring.
        let expanded = if let Some(rest) = path.strip_prefix("~/") {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(rest))
        } else {
            Some(std::path::PathBuf::from(&path))
        };
        if expanded
            .as_ref()
            .is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.is_dir()))
            && let Some(Overlay::Attachment {
                editor,
                selected,
                error,
                ..
            }) = &mut self.view.overlay
        {
            let mut next = path;
            if !next.ends_with('/') {
                next.push('/');
            }
            *editor = Editor::new(next);
            *selected = 0;
            *error = None;
            return;
        }
        self.attach_file_path(path, effects);
    }

    fn attach_file_path(&mut self, path: String, effects: &mut Vec<Effect>) {
        let (Some(account), Some(chat)) = (self.view.account.clone(), self.view.chat.clone())
        else {
            return;
        };
        let request = self.request();
        if let Some(Overlay::Attachment {
            importing, error, ..
        }) = &mut self.view.overlay
        {
            *importing = Some(request);
            *error = None;
        }
        if is_image_path(&path) {
            effects.push(Effect::ImportImage {
                request,
                account,
                chat,
                path,
            });
        } else {
            effects.push(Effect::ImportDocument {
                request,
                account,
                chat,
                path,
            });
        }
    }
    pub(super) fn image_imported(
        &mut self,
        request: RequestId,
        account: AccountId,
        chat: ChatId,
        result: Result<crate::media::outgoing::LocalImage, String>,
    ) {
        if self.quitting
            || self.view.account.as_ref() != Some(&account)
            || self.view.chat.as_ref() != Some(&chat)
        {
            return;
        }
        let Some(Overlay::Attachment {
            importing, error, ..
        }) = &mut self.view.overlay
        else {
            return;
        };
        if *importing != Some(request) {
            return;
        }
        *importing = None;
        match result {
            Ok(image) => {
                self.view.draft.attachment = Some(Box::new(image));
                self.view.draft.document = None;
                self.view.overlay = None;
                self.view.notice = None;
                self.remember();
            }
            Err(reason) => *error = Some(reason),
        }
    }
    pub(super) fn document_imported(
        &mut self,
        request: RequestId,
        account: AccountId,
        chat: ChatId,
        result: Result<crate::media::outgoing::LocalDocument, String>,
    ) {
        if self.quitting
            || self.view.account.as_ref() != Some(&account)
            || self.view.chat.as_ref() != Some(&chat)
        {
            return;
        }
        let Some(Overlay::Attachment {
            importing, error, ..
        }) = &mut self.view.overlay
        else {
            return;
        };
        if *importing != Some(request) {
            return;
        }
        *importing = None;
        match result {
            Ok(file) => {
                self.view.draft.document = Some(Box::new(file));
                self.view.draft.attachment = None;
                self.view.overlay = None;
                self.view.notice = None;
                self.remember();
            }
            Err(reason) => *error = Some(reason),
        }
    }
}

pub(crate) fn clean_attachment_path(raw: &str) -> String {
    let mut path = raw.trim().to_owned();
    // Terminal drag-and-drop typically bracket-pastes the path, often quoted.
    if path.len() >= 2
        && ((path.starts_with('\'') && path.ends_with('\''))
            || (path.starts_with('"') && path.ends_with('"')))
    {
        path = path[1..path.len() - 1].to_owned();
        path = path.trim().to_owned();
    }
    // Unescape shell-style `\ ` produced by some terminals.
    if path.contains("\\ ") {
        path = path.replace("\\ ", " ");
    }
    // Accept file:// URIs dropped from file managers.
    if let Ok(url) = reqwest::Url::parse(&path)
        && let Some(file) = url.to_file_path().ok()
        && let Some(s) = file.to_str()
    {
        return s.to_owned();
    }
    path
}

fn is_image_path(path: &str) -> bool {
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp")
}

#[derive(Clone, Debug)]
pub(crate) struct ExplorerEntry {
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

pub(crate) fn browser_cwd() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
    })
}

fn expand_base(input: &str) -> std::path::PathBuf {
    if let Some(rest) = input.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(input)
}

/// Base directory and filename prefix derived from the typed path.
/// Returns the directory to list and the partial name to filter by.
pub(crate) fn browser_base(input: &str) -> (std::path::PathBuf, String) {
    let cleaned = clean_attachment_path(input);
    if cleaned.is_empty() {
        return (browser_cwd(), String::new());
    }
    let path = expand_base(&cleaned);
    if cleaned.ends_with('/') {
        return (path, String::new());
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
        return (path, String::new());
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            if std::fs::metadata(parent).is_ok_and(|m| m.is_dir()) {
                let prefix = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                return (parent.to_owned(), prefix);
            }
            (browser_cwd(), cleaned)
        }
        _ => (browser_cwd(), cleaned),
    }
}

pub(crate) fn browser_entries(base: &std::path::Path) -> Vec<ExplorerEntry> {
    let mut entries = Vec::new();
    let Ok(read) = std::fs::read_dir(base) else {
        return entries;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." {
            continue;
        }
        let path = entry.path();
        let (is_dir, size) = entry
            .metadata()
            .map(|m| (m.is_dir(), m.len()))
            .unwrap_or((false, 0));
        entries.push(ExplorerEntry {
            name,
            path,
            is_dir,
            size,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    // Always offer `..` first so keyboard users can go up without typing.
    if let Some(parent) = base.parent() {
        entries.insert(
            0,
            ExplorerEntry {
                name: "..".into(),
                path: parent.to_owned(),
                is_dir: true,
                size: 0,
            },
        );
    }
    entries.truncate(100);
    entries
}

/// File list for the attach dialog. `None` means show saved drafts instead.
pub(crate) fn browser_visible(editor_text: &str, has_recovered: bool) -> bool {
    !(editor_text.trim().is_empty() && has_recovered)
}

pub(crate) fn browser_listed(editor_text: &str) -> (std::path::PathBuf, Vec<ExplorerEntry>) {
    let (base, prefix) = browser_base(editor_text);
    let mut entries = browser_entries(&base);
    if !prefix.is_empty() {
        let needle = prefix.to_lowercase();
        entries.retain(|e| e.name.to_lowercase().starts_with(&needle) || e.name == "..");
    }
    (base, entries)
}

pub(crate) fn format_entry_size(size: u64) -> String {
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", size as f64 / (1024.0 * 1024.0))
    }
}

impl App {
    pub(super) fn paste_clipboard(&mut self, effects: &mut Vec<Effect>) {
        if self.view.loading || self.clipboard_request.is_some() {
            return;
        }
        let (Some(account), Some(chat)) = (self.view.account.clone(), self.view.chat.clone())
        else {
            return;
        };
        let sticker = matches!(self.view.overlay, Some(Overlay::Stickers(_)));
        if let Some(Overlay::Stickers(picker)) = &self.view.overlay
            && (picker.loading.is_some() || picker.sending.is_some())
        {
            return;
        }
        let request = self.request();
        self.clipboard_request = Some((request, account.clone(), chat.clone(), sticker));
        self.view.notice = Some("Reading clipboard…".into());
        effects.push(Effect::PasteClipboard {
            request,
            account,
            chat,
            sticker,
        });
    }
    pub(super) fn clipboard_read(
        &mut self,
        request: RequestId,
        account: AccountId,
        chat: ChatId,
        result: Result<crate::desktop::clipboard::Paste, String>,
    ) {
        let Some((pending, expected_account, expected_chat, sticker)) =
            self.clipboard_request.clone()
        else {
            return;
        };
        if self.quitting
            || pending != request
            || expected_account != account
            || expected_chat != chat
        {
            return;
        }
        self.clipboard_request = None;
        if self.view.account.as_ref() != Some(&account) || self.view.chat.as_ref() != Some(&chat) {
            return;
        }
        if sticker {
            self.view.notice = None;
            let Some(Overlay::Stickers(picker)) = &mut self.view.overlay else {
                return;
            };
            match result {
                Ok(crate::desktop::clipboard::Paste::Image(image)) if image.sticker.is_some() => {
                    picker.items.insert(0, StickerChoice::Local(image));
                    picker.items.truncate(60);
                    picker.selected = 0;
                    picker.error = None;
                    self.view.notice = Some("Sticker ready · Enter sends · Esc cancels".into());
                }
                Ok(_) => picker.error = Some("Copy an image to create a sticker".into()),
                Err(reason) => picker.error = Some(reason),
            }
            return;
        }
        match result {
            Ok(crate::desktop::clipboard::Paste::Image(image)) => {
                self.view.draft.attachment = Some(image);
                self.remember();
                self.view.notice =
                    Some("Image attached · type a caption, then Enter to send".into());
            }
            Ok(crate::desktop::clipboard::Paste::Text(text)) => {
                self.edit_current(EditAction::Insert(text));
                self.view.notice = None;
            }
            Err(reason) => self.view.notice = Some(reason),
        }
    }
}
