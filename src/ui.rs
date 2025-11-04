use std::{collections::VecDeque, mem, path::PathBuf, sync::Arc};

use iced::{
    Length::{self, Fill},
    Task,
    alignment::Vertical,
    task::sipper,
    widget::{button, column, container, grid, row, scrollable, text, text_input},
};
use rfd::{AsyncFileDialog, FileHandle};
use tokio::io::{AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub enum Message {
    SelectFile,
    SelectedFolder(Option<Arc<FileHandle>>),
    AbortScan,
    ScanComplete,
    Error(String),
    SearchChanged(String),
    SeperatorChanged(String),
    StartScan,
    ScanUpdate {
        now_scanned: u64,
        occurences: Vec<Occurence>,
    },
    ExportCsv,
    CsvExportComplete(Result<String, String>),
}

pub struct UI {
    selecting: bool,
    selected: Option<PathBuf>,
    cancellation_token: Option<CancellationToken>,
    paths_over_limit: Vec<Occurence>,
    scanned: u64,
    search_string: String,
    running_search_string: String,
    seperator: char,
    running_seperator: char,
    errors: Vec<String>,
    exporting: bool,
    export_message: Option<String>,
    export_success: bool,
}

#[derive(Debug, Clone)]
pub struct Occurence {
    line_number: u64,
    line_character_offset: u64,
    line_byte_offset: u64,
    total_byte_offset: u64,
}

impl UI {
    pub fn start() -> (Self, Task<Message>) {
        (
            Self {
                selecting: false,
                selected: None,
                cancellation_token: None,
                paths_over_limit: Vec::new(),
                scanned: 0,
                search_string: String::new(),
                running_search_string: String::new(),
                seperator: ',',
                running_seperator: ',',
                errors: Vec::new(),
                exporting: false,
                export_message: None,
                export_success: false,
            },
            Task::none(),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectFile => {
                self.selecting = true;
                Task::future(async {
                    let folder = AsyncFileDialog::new().pick_file().await;
                    Message::SelectedFolder(folder.map(Arc::new))
                })
            }
            Message::SelectedFolder(selected) => {
                self.selecting = false;
                if let Some(selected) = selected {
                    if let Some(selected) = Arc::into_inner(selected) {
                        let selected: PathBuf = selected.path().into();
                        self.selected = Some(selected.clone());
                    }
                }
                Task::none()
            }
            Message::AbortScan => {
                if let Some(token) = self.cancellation_token.take() {
                    token.cancel();
                }
                Task::none()
            }
            Message::ScanComplete => {
                if let Some(token) = self.cancellation_token.take() {
                    token.cancel();
                }
                Task::none()
            }
            Message::Error(err) => {
                self.errors.push(err);
                Task::none()
            }
            Message::SearchChanged(new_search) => {
                self.search_string = new_search.clone();
                Task::none()
            }
            Message::SeperatorChanged(new_seperator) => {
                for new_seperator in new_seperator.chars() {
                    if self.seperator != new_seperator {
                        self.seperator = new_seperator;
                        break;
                    }
                }
                Task::none()
            }
            Message::StartScan => {
                if let Some(ref folder) = self.selected {
                    self.paths_over_limit.clear();
                    self.errors.clear();
                    self.scanned = 0;
                    self.export_message = None;
                    let token = CancellationToken::new();
                    self.cancellation_token = Some(token.clone());
                    self.running_search_string = self.search_string.clone();
                    self.running_seperator = self.seperator.clone();
                    self.start_scan(
                        folder.clone(),
                        self.running_search_string.clone(),
                        self.running_seperator.clone(),
                        token,
                    )
                } else {
                    Task::none()
                }
            }
            Message::ScanUpdate {
                now_scanned,
                occurences: new_paths_over_limit,
            } => {
                self.scanned = now_scanned;
                self.paths_over_limit.extend(new_paths_over_limit);
                Task::none()
            }
            Message::ExportCsv => {
                if self.paths_over_limit.is_empty() {
                    Task::none()
                } else {
                    self.exporting = true;
                    self.export_message = None;
                    let paths_to_export = self.paths_over_limit.clone();
                    Task::future(async move {
                        let file_handle = AsyncFileDialog::new()
                            .set_file_name("occurences.csv")
                            .add_filter("CSV", &["csv"])
                            .save_file()
                            .await;

                        if let Some(file_handle) = file_handle {
                            let export_count = paths_to_export.len();
                            let file_path = file_handle.path().to_path_buf();

                            match tokio::fs::File::create(&file_path).await {
                                Ok(mut file) => {
                                    use tokio::io::AsyncWriteExt;

                                    // Write CSV header
                                    if let Err(e) =
                                        file.write_all(b"Byte offset,Line,Char offset in line, Byte offset in line\n").await
                                    {
                                        return Message::CsvExportComplete(Err(format!(
                                            "Failed to write CSV header: {}",
                                            e
                                        )));
                                    }

                                    // Write in chunks of 1000 lines
                                    for chunk in paths_to_export.chunks(1000) {
                                        let mut chunk_content = String::new();
                                        for occurence in chunk {
                                            chunk_content.push_str(&format!(
                                                "{},{},{},{}\n",
                                                occurence.total_byte_offset,
                                                occurence.line_number,
                                                occurence.line_character_offset,
                                                occurence.line_byte_offset
                                            ));
                                        }

                                        if let Err(e) =
                                            file.write_all(chunk_content.as_bytes()).await
                                        {
                                            return Message::CsvExportComplete(Err(format!(
                                                "Failed to write CSV chunk: {}",
                                                e
                                            )));
                                        }
                                    }

                                    if let Err(e) = file.flush().await {
                                        return Message::CsvExportComplete(Err(format!(
                                            "Failed to flush CSV file: {}",
                                            e
                                        )));
                                    }

                                    Message::CsvExportComplete(Ok(format!(
                                        "Exported {} paths to {}",
                                        export_count,
                                        file_path.display()
                                    )))
                                }
                                Err(e) => Message::CsvExportComplete(Err(format!(
                                    "Failed to create CSV file: {}",
                                    e
                                ))),
                            }
                        } else {
                            Message::CsvExportComplete(Err("Export cancelled".to_string()))
                        }
                    })
                }
            }
            Message::CsvExportComplete(result) => {
                self.exporting = false;
                match result {
                    Ok(success_msg) => {
                        self.export_message = Some(success_msg);
                        self.export_success = true;
                        Task::none()
                    }
                    Err(error_msg) => {
                        self.export_message = Some(error_msg);
                        self.export_success = false;
                        Task::none()
                    }
                }
            }
        }
    }

    pub fn view(&'_ self) -> iced::Element<'_, Message> {
        let main_controls = column![
            row![
                button(text("Select File")).on_press_maybe(if self.selecting {
                    None
                } else {
                    Some(Message::SelectFile)
                }),
                if let Some(selected) = &self.selected {
                    text(selected.to_string_lossy())
                } else {
                    text("")
                }
            ]
            .spacing(10)
            .align_y(Vertical::Center),
            row![
                text("Search String:").width(200),
                text_input("", &self.search_string)
                    .on_input(Message::SearchChanged)
                    .on_submit(Message::StartScan)
                    .width(Length::Fill),
            ]
            .spacing(10)
            .align_y(Vertical::Center),
            row![
                text("Seperator:").width(200),
                text_input("", &self.seperator.to_string())
                    .on_input(Message::SeperatorChanged)
                    .on_submit(Message::StartScan)
                    .width(Length::Fill),
            ]
            .spacing(10)
            .align_y(Vertical::Center),
            row![
                button(text("Start Scan")).on_press_maybe(
                    if self.selected.is_some()
                        && !self.cancellation_token.is_some()
                        && !self.search_string.is_empty()
                    {
                        Some(Message::StartScan)
                    } else {
                        None
                    }
                ),
                button(text("Abort")).on_press_maybe(if self.cancellation_token.is_some() {
                    Some(Message::AbortScan)
                } else {
                    None
                }),
                button(text("Export CSV")).on_press_maybe(
                    if !self.paths_over_limit.is_empty()
                        && !self.exporting
                        && self.cancellation_token.is_none()
                    {
                        Some(Message::ExportCsv)
                    } else {
                        None
                    }
                ),
            ]
            .spacing(10),
        ]
        .spacing(10);

        let mut content = column![main_controls].spacing(20);

        if self.cancellation_token.is_some() {
            content =
                content.push(text(format!("Scanning... {} bytes searched", self.scanned)).size(16));
        }

        if !self.paths_over_limit.is_empty() {
            let results_title = text(format!(
                "Found {} occurences of \"{}\"",
                self.paths_over_limit.len(),
                self.running_search_string
            ))
            .size(18);

            content = content.push(results_title);
        }

        if self.exporting {
            content = content.push(text("Exporting to CSV...").size(16));
        }

        if let Some(ref message) = self.export_message {
            let export_text = if self.export_success {
                text(message)
                    .size(16)
                    .color(iced::Color::from_rgb(0.0, 0.6, 0.0))
            } else {
                text(message)
                    .size(16)
                    .color(iced::Color::from_rgb(0.8, 0.2, 0.2))
            };
            content = content.push(export_text);
        }

        if !self.errors.is_empty() {
            let errors_title = text(format!("Errors ({})", self.errors.len()))
                .size(18)
                .color(iced::Color::from_rgb(0.8, 0.2, 0.2));

            let errors_list =
                scrollable(column(self.errors.iter().map(|error| text(error).into())))
                    .height(Length::Fill)
                    .width(Length::Fill);

            content = content.push(errors_title).push(errors_list);
        }

        content.padding(20).into()
    }

    fn start_scan(
        &mut self,
        root: PathBuf,
        search_string: String,
        seperator: char,
        token: CancellationToken,
    ) -> Task<Message> {
        let sipper = sipper(move |mut sender| async move {
            let mut occurences: Vec<Occurence> = Vec::new();

            let file = match tokio::fs::File::open(root.as_path()).await {
                Ok(file) => file,
                Err(err) => {
                    sender.send(Message::Error(err.to_string())).await;
                    return;
                }
            };
            let mut reader = BufReader::with_capacity(1024 * 1024, file);

            // The characters we're searching for
            let search_chars = search_string.to_lowercase().chars().collect::<Vec<_>>();
            // Which line we're currently on
            let mut line_number = 1u64;
            // Which character we're currently on in the line
            let mut line_character_offset = 0u64;
            // Which byte that character is at
            let mut line_byte_offset = 0u64;
            // Which byte that character is at in total
            let mut total_byte_offset = 0u64;
            // Buffer to store already read characters
            let mut found = VecDeque::<char>::new();
            // which char in the search_chars we're currently on
            let mut compare_index = 0;
            let mut last_update_sent_bytes = 0u64;

            token
                .run_until_cancelled(async move {
                    // reserved space for a single character
                    let mut unicode_character_bytes = [0u8; 4];
                    loop {
                        // send periodic updates to GUI
                        if total_byte_offset - last_update_sent_bytes > 1024 * 1024 {
                            sender
                                .send(Message::ScanUpdate {
                                    now_scanned: total_byte_offset,
                                    occurences: mem::take(&mut occurences),
                                })
                                .await;
                            last_update_sent_bytes = total_byte_offset;
                        }

                        // read the first byte of the character
                        let first_byte = match reader.read_u8().await {
                            Ok(byte) => byte,
                            Err(err) => {
                                if err.kind() == std::io::ErrorKind::UnexpectedEof {
                                    break;
                                }
                                sender.send(Message::Error(err.to_string())).await;
                                return;
                            }
                        };

                        // check how many bytes are needed for the character
                        let len = match utf8_char_len(first_byte) {
                            Some(len) => len,
                            None => {
                                sender
                                    .send(Message::Error("Invalid UTF-8 sequence".to_string()))
                                    .await;
                                return;
                            }
                        };

                        // how many characters in we are
                        line_character_offset += 1;
                        // how many bytes that character is at
                        line_byte_offset += len as u64;
                        // Which byte that character is at in total
                        total_byte_offset += len as u64;

                        unicode_character_bytes[0] = first_byte;
                        if len > 1 {
                            match reader
                                .read_exact(&mut unicode_character_bytes[1..len])
                                .await
                            {
                                Ok(_) => (),
                                Err(err) => {
                                    sender.send(Message::Error(err.to_string())).await;
                                    return;
                                }
                            }
                        }

                        let str = match std::str::from_utf8(&unicode_character_bytes[..len]) {
                            Ok(s) => s,
                            Err(err) => {
                                sender.send(Message::Error(err.to_string())).await;
                                return;
                            }
                        };

                        let char = str.chars().next().unwrap().to_lowercase().next().unwrap();

                        match char {
                            '\n' => {
                                line_number += 1;
                                line_character_offset = 0;
                                line_byte_offset = 0;
                                compare_index = 0;
                                found.clear();
                            }
                            char => {
                                if char == seperator {
                                    compare_index = 0;
                                    found.clear();
                                }
                            }
                        }

                        if char == search_chars[compare_index] {
                            found.push_back(char);
                            compare_index += 1;
                            if compare_index >= search_chars.len() {
                                occurences.push(Occurence {
                                    line_number,
                                    line_character_offset,
                                    line_byte_offset,
                                    total_byte_offset,
                                });
                            } else {
                                continue;
                            }
                        }

                        if found.len() == 0 {
                            continue;
                        }

                        found.pop_front();

                        compare_index = 0;

                        while found.len() > 0 {
                            if search_chars[compare_index] == found[compare_index] {
                                compare_index += 1;
                                if compare_index >= found.len() {
                                    break;
                                }
                            } else {
                                compare_index = 0;
                                found.pop_front();
                            }
                        }

                        found.clear();
                    }

                    sender
                        .send(Message::ScanUpdate {
                            now_scanned: total_byte_offset,
                            occurences: mem::take(&mut occurences),
                        })
                        .await;
                })
                .await;
        });

        Task::sip(sipper, |value| value, |_| Message::ScanComplete)
    }
}

fn utf8_char_len(first: u8) -> Option<usize> {
    if first & 0b1000_0000 == 0 {
        Some(1) // 0xxxxxxx
    } else if first & 0b1110_0000 == 0b1100_0000 {
        Some(2) // 110xxxxx
    } else if first & 0b1111_0000 == 0b1110_0000 {
        Some(3) // 1110xxxx
    } else if first & 0b1111_1000 == 0b1111_0000 {
        Some(4) // 11110xxx
    } else {
        None // continuation byte or invalid leading byte
    }
}
