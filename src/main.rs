#![windows_subsystem = "windows"] // Prevents terminal window from opening
use eframe::egui;
use image::ImageFormat;

use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread,
};

// Imports for parallel search
use rayon::prelude::*;
use walkdir::WalkDir;

// --- App Structure and Initialization ---

pub struct MyExplorerApp {
    pub current_dir: PathBuf,
    pub entries: Vec<String>,
    pub filtered_entries: Option<Vec<String>>,
    pub recursive_search_results: Option<Vec<PathBuf>>,
    pub rename_mode: Option<String>,
    pub rename_input: String,
    pub show_search_popup: bool,
    pub search_query: String,
    pub search_sender: Option<mpsc::Sender<Vec<PathBuf>>>,
    pub search_receiver: Option<mpsc::Receiver<Vec<PathBuf>>>,
    pub is_searching: bool,
    pub app_icon: Option<egui::ColorImage>, // For in-app display
}

impl Default for MyExplorerApp {
    fn default() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(""));
        let mut app = Self {
            current_dir,
            entries: Vec::new(),
            filtered_entries: None,
            recursive_search_results: None,
            rename_mode: None,
            rename_input: String::new(),
            show_search_popup: false,
            search_query: String::new(),
            search_sender: None,
            search_receiver: None,
            is_searching: false,
            app_icon: load_egui_image_from_bytes(include_bytes!("./icon.png")),
        };

        app.read_current_directory_entries();
        app
    }
}

// --- App Logic Methods ---

impl MyExplorerApp {
    /// Reads the entries of the current directory and updates `self.entries`.
    /// Also resets all search results.
    fn read_current_directory_entries(&mut self) {
        self.entries.clear();
        match std::fs::read_dir(&self.current_dir) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if entry.path().is_dir() {
                            self.entries.push(format!("{}/", name)); // Mark folder with slash
                        } else {
                            self.entries.push(name);
                        }
                    }
                }
                self.entries.sort_unstable(); // Sort for better display
            }
            Err(e) => {
                eprintln!("Error while loading directory {:?}: {}", self.current_dir, e);
                // Optionally: show error in UI
            }
        }
        self.filtered_entries = None; // Reset filtering for current directory
        self.recursive_search_results = None; // Reset recursive search results
        self.is_searching = false; // Stop searching if directory changes
        self.search_sender = None; // Close channels
        self.search_receiver = None; // Close channels
    }

    /// Navigates into a subfolder.
    fn navigate_to(&mut self, entry_name: &str) {
        let mut new_path = self.current_dir.clone();
        new_path.push(entry_name);
        if new_path.is_dir() {
            self.current_dir = new_path;
            self.read_current_directory_entries(); // Reload entries and reset search
        }
    }

    /// Navigates to the parent directory.
    fn navigate_up(&mut self) {
        if self.current_dir.parent().is_some() {
            self.current_dir.pop();
            self.read_current_directory_entries(); // Reload entries and reset search
        }
    }

    /// Renames an entry.
    fn rename_entry(&mut self, old_name: &str, new_name: &str) {
        let mut old_path = self.current_dir.clone();
        old_path.push(old_name);
        let mut new_path = self.current_dir.clone();
        new_path.push(new_name);

        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            eprintln!("Error while renaming {:?} to {:?}: {}", old_path, new_path, e);
            // Optionally: show error in UI
        } else {
            self.read_current_directory_entries(); // Update entries after renaming and reset search
        }
    }

    /// Deletes an entry (file or folder).
    fn delete_entry(&mut self, entry_name: &str) {
        let mut path_to_delete = self.current_dir.clone();
        path_to_delete.push(entry_name);

        let result = if path_to_delete.is_dir() {
            std::fs::remove_dir_all(&path_to_delete)
        } else {
            std::fs::remove_file(&path_to_delete)
        };

        if let Err(e) = result {
            eprintln!("Error while deleting {:?}: {}", path_to_delete, e);
            // Optionally: show error in UI
        } else {
            self.read_current_directory_entries(); // Update entries after deletion and reset search
        }
    }

    /// Recursively searches from `start_path` for entries containing `query_lower`.
    /// Uses `rayon` for parallelization.
    fn find_entries_recursively(
        start_path: &PathBuf,
        query_lower: &str,
    ) -> Vec<PathBuf> {
        WalkDir::new(start_path)
            .into_iter()
            .filter_map(|e| e.ok()) // Skip entries with errors
            .par_bridge() // Parallelize iteration using rayon
            .filter_map(|entry| {
                let path = entry.path();
                let file_name = path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                // Check if current entry (file or folder name) contains the search term (case-insensitive)
                if file_name.to_lowercase().contains(query_lower) {
                    Some(path.to_owned())
                } else {
                    None
                }
            })
            .collect() // Collect all results into a Vec
    }

    /// Executes the recursive search based on `self.search_query`
    /// and saves the results in `self.recursive_search_results`.
    /// This function starts a new thread for searching, with rayon parallelization inside.
    fn execute_search(&mut self, ctx: egui::Context) {
        let query_lower = self.search_query.to_lowercase();
        if query_lower.is_empty() {
            self.recursive_search_results = None;
            self.is_searching = false; // Reset search status
            return;
        }

        // Create new channel for this search operation
        let (sender, receiver) = mpsc::channel();
        self.search_sender = Some(sender.clone());
        self.search_receiver = Some(receiver);
        self.is_searching = true;
        self.recursive_search_results = None; // Immediately clear old results

        let current_dir_for_thread = self.current_dir.clone();
        let search_query_for_thread = query_lower.clone(); // Clone for thread

        // Start a new thread for the search
        // Rayon handles parallelization *within* this thread
        thread::spawn(move || {
            let found_paths = Self::find_entries_recursively(&current_dir_for_thread, &search_query_for_thread);
            if sender.send(found_paths).is_ok() {
                ctx.request_repaint(); // Request repaint in main thread when results sent
            }
            // Sender is automatically dropped when thread ends
        });
    }
}

// Helper function to load PNG bytes into egui::ColorImage (for in-app display)
fn load_egui_image_from_bytes(bytes: &'static [u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png).ok()?;
    let size = [image.width() as _, image.height() as _];
    let image_buffer = image.into_rgba8();
    let pixels = image_buffer.as_flat_samples();
    Some(egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()))
}

// --- Egui/Eframe Implementation ---

impl eframe::App for MyExplorerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Flags for delayed state changes
        let mut should_navigate_to_path: Option<PathBuf> = None;
        let mut should_clear_recursive_results_after_interaction = false;
        let mut should_clear_rename_mode = false;
        let mut should_close_search_popup = false;

        // Check for search results from background thread
        if let Some(receiver) = &self.search_receiver {
            match receiver.try_recv() {
                Ok(results) => {
                    self.recursive_search_results = Some(results);
                    self.is_searching = false; // Search finished
                    self.search_sender = None; // Close channels
                    self.search_receiver = None; // Close channels
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // No results yet, search still running
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Sender dropped, search ended or failed
                    self.is_searching = false;
                    self.search_sender = None;
                    self.search_receiver = None;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(icon) = &self.app_icon {
                    let texture_id = ctx.tex_manager().write().alloc(
                        "app_logo".to_owned(),
                        egui::ImageData::Color(icon.clone()),
                        egui::TextureOptions::default(),
                    );
                    ui.image(texture_id, egui::vec2(24.0, 24.0)); // Adjust size
                }
                ui.heading("FileFox");
            });

            // --- Navigation bar ---
            ui.horizontal(|ui| {
                if ui.button("⬆️ Up").clicked() {
                    self.navigate_up();
                }
                ui.label(format!("Current Path: {}", self.current_dir.display()));
            });

            ui.separator();

            // Loading indicator when searching
            if self.is_searching {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Searching for: '{}'...", self.search_query));
                });
                ui.separator();
            }

            // --- Display file entries / search results ---
            let display_mode_is_recursive_search = self.recursive_search_results.is_some();

            egui::ScrollArea::vertical().show(ui, |ui| {
                if display_mode_is_recursive_search {
                    // Show recursive search results
                    let results_cloned = self.recursive_search_results.clone().unwrap_or_default();

                    if results_cloned.is_empty() {
                        ui.label(format!("No results found for: '{}'", self.search_query));
                    } else {
                        ui.heading(format!("Results for: '{}'", self.search_query));
                        ui.add_space(10.0);

                        for path in &results_cloned {
                            let path_str = path.display().to_string();
                            let response = ui.button(&path_str);

                            // Double click: navigate or open
                            if response.double_clicked() {
                                if path.is_dir() {
                                    should_navigate_to_path = Some(path.clone());
                                    should_clear_recursive_results_after_interaction = true;
                                } else {
                                    let _ = Command::new("cmd")
                                        .args(&["/C", "start", "", &path_str])
                                        .spawn();
                                }
                            }
                            // Right-click context menu for search results
                            response.context_menu(|ui| {
                                if ui.button("Open").clicked() {
                                    if path.is_dir() {
                                        should_navigate_to_path = Some(path.clone());
                                        should_clear_recursive_results_after_interaction = true;
                                    } else {
                                        let _ = Command::new("cmd")
                                            .args(&["/C", "start", "", &path_str])
                                            .spawn();
                                    }
                                    ui.close_menu();
                                }
                                if ui.button("Show in explorer").clicked() {
                                    let _ = Command::new("explorer")
                                        .args(&["/select,", &path_str])
                                        .spawn();
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                } else {
                    // Normal view of entries in current directory
                    let entries_to_display_cloned: Vec<String> = if let Some(filtered) = &self.filtered_entries {
                        filtered.clone()
                    } else {
                        self.entries.clone()
                    };

                    for entry in &entries_to_display_cloned {
                        let is_dir = entry.ends_with('/');
                        let entry_name = if is_dir {
                            entry.trim_end_matches('/').to_string()
                        } else {
                            entry.clone()
                        };

                        // --- Rename mode ---
                        if self.rename_mode.as_deref() == Some(&entry_name) {
                            ui.horizontal(|ui| {
                                let text_edit = ui.text_edit_singleline(&mut self.rename_input);
                                if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    if !self.rename_input.is_empty() {
                                        let new_name = self.rename_input.clone();
                                        self.rename_entry(&entry_name, &new_name);
                                    }
                                    should_clear_rename_mode = true; // Delayed reset
                                }
                                if ui.button("Cancel").clicked() {
                                    should_clear_rename_mode = true; // Delayed reset
                                }
                            });
                        }
                        // --- Normal entry ---
                        else {
                            let response = ui.button(entry);

                            // Double click: navigate folder, open file
                            if response.double_clicked() {
                                if is_dir {
                                    self.navigate_to(&entry_name);
                                } else {
                                    let mut path = self.current_dir.clone();
                                    path.push(&entry_name);
                                    let _ = Command::new("cmd")
                                        .args(&["/C", "start", "", &path.to_string_lossy()])
                                        .spawn();
                                }
                            }

                            // Right-click context menu
                            response.context_menu(|ui| {
                                if ui.button("Open").clicked() {
                                    if is_dir {
                                        self.navigate_to(&entry_name);
                                    } else {
                                        let mut path = self.current_dir.clone();
                                        path.push(&entry_name);
                                        let _ = Command::new("cmd")
                                            .args(&["/C", "start", "", &path.to_string_lossy()])
                                            .spawn();
                                    }
                                    ui.close_menu();
                                }

                                if ui.button("Delete").clicked() {
                                    self.delete_entry(&entry_name);
                                    ui.close_menu();
                                }

                                if ui.button("Rename").clicked() {
                                    self.rename_mode = Some(entry_name.clone());
                                    self.rename_input = entry_name.clone();
                                    ui.close_menu();
                                }

                                if ui.button("Search").clicked() {
                                    self.show_search_popup = true; // Show search popup
                                    self.search_query.clear(); // Clear search field when opening
                                    self.recursive_search_results = None; // Clear old search results
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                }
            });
        });

        // --- Render search popup ---

        if self.show_search_popup {
            egui::Window::new("What do you want to search?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    let response = ui.text_edit_singleline(&mut self.search_query);

                    ui.horizontal(|ui| {
                        // "Search" button disabled if already searching
                        ui.add_enabled_ui(!self.is_searching, |ui| {
                            if ui.button("Search").clicked() {
                                self.execute_search(ctx.clone());
                                should_close_search_popup = true; // Close popup after starting search
                            }
                        });
                        if ui.button("Cancel").clicked() {
                            self.recursive_search_results = None; // Clear results on cancel
                            self.is_searching = false; // Stop search
                            self.search_sender = None; // Close channels
                            self.search_receiver = None; // Close channels
                            should_close_search_popup = true;
                        }
                    });

                    // Trigger search on enter key if text box focused and not already searching
                    if !self.is_searching && response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.execute_search(ctx.clone());
                        should_close_search_popup = true; // Close popup after starting search
                    }
                });
        }

        // --- Apply delayed state changes ---
        if should_close_search_popup {
            self.show_search_popup = false;
        }
        if let Some(path_to_navigate) = should_navigate_to_path {
            self.current_dir = path_to_navigate;
            self.read_current_directory_entries();
        }
        if should_clear_recursive_results_after_interaction {
            self.recursive_search_results = None;
        }
        if should_clear_rename_mode {
            self.rename_mode = None;
        }
    }
}

// --- Main function to start the application ---

fn main() {
    // Load PNG bytes directly for window icon
    let window_icon_data = eframe::IconData::try_from_png_bytes(
        include_bytes!("./icon.png")
    ).ok();

    let mut native_options = eframe::NativeOptions::default();
    // Set window icon if loaded
    if let Some(icon_data) = window_icon_data {
        native_options.icon_data = Some(icon_data);
    }

    let _ = eframe::run_native(
        "FileFox", // Application name
        native_options,
        Box::new(|_cc| Box::new(MyExplorerApp::default())), // Expected closure
    );
}